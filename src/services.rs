use std::{
    sync::{
        mpsc::{channel, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

use futures::executor::block_on;
use gst::glib::MainContext;
use serde::{Deserialize, Serialize};

use reqwest::Client;
use reqwest_eventsource::{EventSource, RequestBuilderExt};
use tokio::runtime::Runtime;

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
pub struct OutgoingMessage {
    pub recipient: Option<String>,
    pub payload: Payload,
}

#[derive(Clone)]
pub struct Services {
    pub base_url: String,
    pub client: Arc<Mutex<Client>>,
    pub thread_handle: Arc<JoinHandle<()>>,
    pub tx: Arc<Mutex<Sender<OutgoingMessage>>>,
}

#[derive(Debug, Clone)]
pub enum ServiceError {
    Test,
}

impl Services {
    pub fn new(base_url: String) -> Self {
        let client = Arc::new(Mutex::new(
            Client::builder().cookie_store(true).build().unwrap(),
        ));
        let (tx, rx) = channel();
        let c = client.clone();
        let b = base_url.clone();
        let thread_handle = Arc::new(thread::spawn(move || {
            let rt = Runtime::new().unwrap();
            while let Ok(msg) = rx.recv() {
                let client = c.lock().unwrap();
                let response = client.post(format!("{}/message", b)).json(&msg).send();

                match rt.block_on(response) {
                    Ok(_response) => {}
                    Err(error) => {
                        eprintln!("Problem sending the message:\n {:?}", error);
                    }
                };
            }
        }));
        let tx = Arc::new(Mutex::new(tx));
        Services {
            base_url,
            client,
            thread_handle,
            tx,
        }
    }

    fn queue_message(&self, message: OutgoingMessage) -> Result<(), ServiceError> {
        let tx = self.tx.lock().unwrap();
        tx.send(message).unwrap();
        Ok(())
    }

    pub fn send_new_camera(&self) -> Result<(), ServiceError> {
        self.queue_message(OutgoingMessage {
            recipient: None,
            payload: Payload::NewCamera,
        })
    }

    pub fn send_camera_ping(&self, recipient: &String) -> Result<(), ServiceError> {
        self.queue_message(OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::CameraPing,
        })
    }

    pub fn send_sdp_offer(
        &self,
        recipient: &String,
        description: String,
    ) -> Result<(), ServiceError> {
        self.queue_message(OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::SDP { description },
        })
    }

    pub fn send_ice_candidate(
        &self,
        recipient: &String,
        index: u32,
        candidate: String,
    ) -> Result<(), ServiceError> {
        self.queue_message(OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::ICE { index, candidate },
        })
    }

    pub fn start_sse(&self) -> EventSource {
        let client = self.client.lock().unwrap();
        client
            .get(format!("{}/events", self.base_url))
            .eventsource()
            .unwrap()
    }
}
