use serde::{Deserialize, Serialize};

use reqwest::Client;
use reqwest_eventsource::{EventSource, RequestBuilderExt};

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Payload {
    Welcome,
    NewCamera,
    CameraDiscovery,
    CameraPing,
    CallInit,
    SDP { description: String },
    ICE { index: u32, candidate: String },
}

// Incoming Messages
#[derive(Clone, Deserialize, Debug)]
pub struct IncomingMessage {
    pub sender: String,
    pub payload: Payload,
}

#[derive(Clone, Serialize, Debug)]
struct OutgoingMessage {
    pub recipient: Option<String>,
    pub payload: Payload,
}

#[derive(Clone)]
pub struct Services {
    pub base_url: String,
    pub client: Client,
}

#[derive(Debug, Clone)]
pub enum ServiceError {
    Test,
}

impl Services {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder().cookie_store(true).build().unwrap();
        Services { base_url, client }
    }

    async fn send_message<T: Serialize>(&self, message: &T) -> Result<(), ServiceError> {
        let response = self
            .client
            .post(format!("{}/message", self.base_url))
            .json(&message)
            .send();

        match response.await {
            Ok(_response) => Ok(()),
            Err(error) => {
                eprintln!("Problem sending the message:\n {:?}", error);
                Err(ServiceError::Test)
            }
        }
    }

    pub async fn send_new_camera(&self) -> Result<(), ServiceError> {
        self.send_message(&OutgoingMessage {
            recipient: None,
            payload: Payload::NewCamera,
        })
        .await
    }

    pub async fn send_camera_ping(&self, recipient: &String) -> Result<(), ServiceError> {
        self.send_message(&OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::CameraPing,
        })
        .await
    }

    pub async fn send_sdp_offer(
        &self,
        recipient: &String,
        description: String,
    ) -> Result<(), ServiceError> {
        self.send_message(&OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::SDP { description },
        })
        .await
    }

    pub async fn send_ice_candidate(
        &self,
        recipient: &String,
        index: u32,
        candidate: String,
    ) -> Result<(), ServiceError> {
        self.send_message(&OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::ICE { index, candidate },
        })
        .await
    }

    pub fn start_sse(&self) -> EventSource {
        self.client
            .get(format!("{}/events", self.base_url))
            .eventsource()
            .unwrap()
    }
}
