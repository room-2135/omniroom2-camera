use serde::{Serialize, Deserialize};
use futures::executor::block_on;

use reqwest::Client;
use reqwest_eventsource::RequestBuilderExt;
use futures::stream::StreamExt;

// Incoming Messages

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericIncomingMessage {
    pub command: String,
    pub sender: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SDPAnswerIncomingMessage {
    pub sender: String,
    pub description: String
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ICECandidateIncomingMessage {
    pub sender: String,
    pub index: u32,
    pub candidate: String
}

// Outgoing Messages

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CameraPingOutgoingMessage {
    pub recipient: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SDPOfferOutgoingMessage {
    pub recipient: String,
    pub description: String
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ICECandidateOutgoingMessage {
    pub recipient: String,
    pub index: u32,
    pub candidate: String
}

#[derive(Clone)]
pub struct Services {
    pub base_url: String,
    pub client: Client
}

impl Services {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .cookie_store(true)
            .build()
            .unwrap();
        Services {
            base_url,
            client
        }
    }

    fn send_message<T: Serialize>(&self, url_key: String, message: &T) {
        let response = self.client
            .post(format!("{}/message/{}", self.base_url, url_key))
            .json(&message)
            .send();

        let _response = match block_on(response) {
            Ok(response) => response,
            Err(error) => panic!("Problem sending the message:\n {:?}", error),
        };
    }

    pub fn send_camera_ping(&self, recipient: &String) {
        self.send_message(String::from("camera_ping"), &CameraPingOutgoingMessage {
            recipient: recipient.to_string(),
        });
    }

    pub fn send_sdp_offer(&self, recipient: &String, description: String) {
        self.send_message(String::from("sdp_offer"), &SDPOfferOutgoingMessage {
            recipient: recipient.to_string(),
            description
        });
    }

    pub fn send_ice_candidate(&self, recipient: &String, index: u32, candidate: String) {
        self.send_message(String::from("ice_candidate"), &ICECandidateOutgoingMessage {
            recipient: recipient.to_string(),
            index,
            candidate
        });
    }

    pub async fn start_sse<F1, F2, F3, F4>(&self, on_camera_ping: F1, on_call_init: F2, on_sdp_anwer: F3, on_ice_candidate: F4) where
        F1: Fn(&GenericIncomingMessage),
        F2: Fn(&GenericIncomingMessage),
        F3: Fn(&SDPAnswerIncomingMessage),
        F4: Fn(&ICECandidateIncomingMessage)
    {
        let mut stream = self.client
            .get(format!("{}/events", self.base_url))
            .eventsource()
            .unwrap();

        while let Some(event) = stream.next().await {
            match event {
                Ok(event) => {
                    let message: GenericIncomingMessage = serde_json::from_str(&event.data).expect("JSON was not well-formatted");
                    match message.command.as_str() {
                        "camera_ping" => {
                            on_camera_ping(&message);
                        }
                        "call_init" => {
                            on_call_init(&message);
                        },
                        "sdp_answer" => {
                            let message: SDPAnswerIncomingMessage = serde_json::from_str(&event.data).expect("JSON was not well-formatted");
                            on_sdp_anwer(&message);
                        },
                        "ice_candidate" => {
                            let message: ICECandidateIncomingMessage = serde_json::from_str(&event.data).expect("JSON was not well-formatted");
                            on_ice_candidate(&message);
                        }
                        _ => {
                            eprintln!("Failed to parse received command:\n{:?}", message);
                        }
                    }
                },
                Err(error) => {
                    println!("Error: {:?}", error);
                }
            }
        }
    }
}
