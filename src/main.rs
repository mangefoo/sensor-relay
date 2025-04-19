use std::{collections::HashMap, env};
use std::convert::Infallible;
use std::sync::Arc;
use chrono::Local;
use tokio::sync::{mpsc, RwLock};
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};
use warp::{ws::Message, Filter, Rejection};

mod handler;
mod ws;

type Result<T> = std::result::Result<T, Rejection>;
type Clients = Arc<RwLock<HashMap<String, Client>>>;

#[derive(Debug, Clone)]
pub struct Client {
    pub topics: Vec<String>,
    pub sender: Option<mpsc::UnboundedSender<std::result::Result<Message, warp::Error>>>,
}

fn setup_logging() {
    let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

    struct LocalTimer;
    
    impl fmt::time::FormatTime for LocalTimer {
        fn format_time(&self, w: &mut fmt::format::Writer<'_>) -> std::fmt::Result {
            write!(w, "{}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
        }
    }

    fmt::Subscriber::builder()
        .with_env_filter(EnvFilter::new(log_level))
        .with_timer(LocalTimer)  // Use local time
        .with_target(false)    // Exclude the target
        .with_thread_ids(false)// Exclude thread IDs
        .with_thread_names(false) // Exclude thread names
        .with_file(false)      // Exclude source file info
        .with_line_number(false) // Exclude line numbers
        .compact()             // Use compact formatting
        .init();
    
    info!("Logger initialized with single-line format");
}

#[tokio::main]
async fn main() {

    setup_logging();
    info!("Starting");

    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));

    let health_route = warp::path!("health").and_then(handler::health_handler);

    let register = warp::path("register");
    let register_routes = register
        .and(warp::post())
        .and(warp::body::json())
        .and(with_clients(clients.clone()))
        .and_then(handler::register_handler)
        .or(register
            .and(warp::delete())
            .and(warp::path::param())
            .and(with_clients(clients.clone()))
            .and_then(handler::unregister_handler));

    let publish = warp::path!("publish")
        .and(warp::body::json())
        .and(with_clients(clients.clone()))
        .and_then(handler::publish_handler);

    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(warp::path::param())
        .and(with_clients(clients.clone()))
        .and_then(handler::ws_handler);

    let routes = health_route
        .or(register_routes)
        .or(ws_route)
        .or(publish)
        .with(warp::cors().allow_any_origin());

    warp::serve(routes).run(([0, 0, 0, 0], 8967)).await;
}

fn with_clients(clients: Clients) -> impl Filter<Extract = (Clients,), Error = Infallible> + Clone {
    warp::any().map(move || clients.clone())
}
