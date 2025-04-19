use crate::{ws, Client, Clients, Result};
use rumqttc::AsyncClient;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use uuid::Uuid;
use warp::{http::StatusCode, reply::json, ws::Message, Reply};
use std::{collections::HashMap, sync::{Arc, Mutex, Once}};

static INIT: Once = Once::new();
static MQTT_CLIENT: Mutex<Option<Arc<AsyncClient>>> = Mutex::new(None);

#[derive(Deserialize, Debug)]
pub struct RegisterRequest {
    topics: Vec<String>,
}

#[derive(Serialize, Debug)]
pub struct RegisterResponse {
    id: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct SensorReport {
    reporter: String,
    topic: String,
    sensors: HashMap<String, String>,
}

pub async fn publish_handler(report: SensorReport, clients: Clients) -> Result<impl Reply> {
    let serialized = serde_json::to_string(&report).unwrap();

    clients
        .read()
        .await
        .iter()
        .filter(|(_, client)| client.topics.contains(&report.topic))
        .for_each(|(_, client)| {
            if let Some(sender) = &client.sender {
                let _ = sender.send(Ok(Message::text(serialized.clone())));
            }
        });

    publish_to_mqtt(report).await?;

    Ok(StatusCode::OK)
}

pub async fn publish_to_mqtt(report: SensorReport) -> Result<()> {
    INIT.call_once(|| {
        use rand::Rng;
        let client_id: String = format!("sendor-relay-{}", rand::rng().random::<u32>());
        let mqtt_host = std::env::var("MQTT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let mut mqtt_options = rumqttc::MqttOptions::new(client_id, mqtt_host, 1883);
        mqtt_options.set_keep_alive(std::time::Duration::from_secs(5));
        
        let (client, event_loop) = AsyncClient::new(mqtt_options, 10);
        
        spawn_event_loop(event_loop);
        
        let mut client_guard = MQTT_CLIENT.lock().unwrap();
        *client_guard = Some(Arc::new(client));
    });

    let mqtt_client = {
        let client_guard = MQTT_CLIENT.lock().unwrap();
        client_guard.as_ref().expect("MQTT client not initialized").clone()
    };

    let topic = format!("sensor-relay/{}", report.topic);
    let payload = serde_json::to_string(&report).unwrap();

    if let Err(e) = mqtt_client.publish(topic, rumqttc::QoS::AtLeastOnce, false, payload).await {
        error!("Failed to publish to MQTT broker: {}", e);
        return Err(warp::reject::reject())
    }

    Ok(())
}

fn spawn_event_loop(mut event_loop: rumqttc::EventLoop) {
    tokio::spawn(async move {
        info!("MQTT event loop started");
        loop {
            match event_loop.poll().await {
                Ok(_notification) => {}
                Err(e) => {
                    error!("MQTT event loop error: {:?}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    });
}

pub async fn register_handler(request: RegisterRequest, clients: Clients) -> Result<impl Reply> {
    let uuid = Uuid::new_v4().simple().to_string();

    register_client(uuid.clone(), request.topics, clients).await;
    Ok(json(&RegisterResponse {
        id: uuid
    }))
}

async fn register_client(id: String, topics: Vec<String>, clients: Clients) {
    clients.write().await.insert(
        id,
        Client {
            topics,
            sender: None,
        },
    );
}

pub async fn unregister_handler(id: String, clients: Clients) -> Result<impl Reply> {
    clients.write().await.remove(&id);
    Ok(StatusCode::OK)
}

pub async fn ws_handler(ws: warp::ws::Ws, id: String, clients: Clients) -> Result<impl Reply> {
    let client = clients.read().await.get(&id).cloned();
    match client {
        Some(c) => Ok(ws.on_upgrade(move |socket| ws::client_connection(socket, id, clients, c))),
        None => Err(warp::reject::not_found()),
    }
}

pub async fn health_handler() -> Result<impl Reply> {
    Ok(StatusCode::OK)
}
