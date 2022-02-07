use reqwest::Client;
use serde::{Serialize, Deserialize};
use reqwest_eventsource::RequestBuilderExt;
use reqwest_eventsource::EventsourceRequestBuilder;
use futures::stream::StreamExt;
use futures::executor::block_on;


use gst::prelude::*;
use gst_webrtc::{WebRTCSessionDescription, WebRTCSDPType};
use gst_sdp::sdp_message::SDPMessage;

const URL_SSE: &str = "http://localhost:8000/events";
const URL_MESSAGES: &str = "http://localhost:8000/message";

fn prepare() -> Box<gst::Pipeline> {
    // Initialize gstreamer
    gst::init().unwrap();

    // Build the pipeline
    let pipeline = gst::parse_launch(&format!("videotestsrc ! videoconvert ! videoscale ! video/x-raw,width=1920,height=1080,framerate=30/1 ! x264enc tune=zerolatency ! video/x-h264,profile=baseline ! rtph264pay ! application/x-rtp,media=video,encoding-name=H264,payload=96 ! tee name=videotee ! queue ! fakesink")).unwrap();
    //let pipeline = gst::parse_launch(&format!("v4l2src device=/dev/video1 ! videoconvert ! videoscale ! video/x-raw,width=1920,height=1080,framerate=30/1 ! x264enc tune=zerolatency ! video/x-h264,profile=baseline ! rtph264pay ! application/x-rtp,media=video,encoding-name=H264,payload=96 ! tee name=videotee ! queue ! fakesink")).unwrap();
    Box::new(pipeline.dynamic_cast::<gst::Pipeline>().unwrap())
}

fn add_webrtc(client: &Box<Client>, identifier: String,  pipeline: &Box::<gst::Pipeline>) {
    println!("Adding Webrtc node");

    if let Some(_webrtc) = pipeline.by_name(format!("webrtc-{}", identifier).as_str()) {
        let identifier = identifier.clone();
        remove_webrtc(identifier, pipeline);
    }

    let webrtc = Box::new(gst::ElementFactory::make("webrtcbin", Some(format!("webrtc-{}", identifier).as_str())).expect(""));
    let queue =  gst::ElementFactory::make("queue", Some(format!("queue-{}", identifier).as_str())).expect("");

    pipeline.add_many(&[&queue, &webrtc]).unwrap();

    let srcpad = queue.static_pad("src").unwrap();
    let sinkpad = webrtc.request_pad_simple("sink_%u").unwrap();
    srcpad.link(&sinkpad).unwrap();

    let tee = pipeline.by_name("videotee").unwrap();
    let srcpad = tee.request_pad_simple("src_%u").unwrap();
    let sinkpad = queue.static_pad("sink").unwrap();
    srcpad.link(&sinkpad).unwrap();

    {
        let w = webrtc.clone();
        let client = client.clone();
        let identifier = identifier.clone();
        webrtc.connect("on-negotiation-needed", false, move |_| {
            println!("Negociation needed");
            on_negotiation_needed(&client, identifier.clone(), &w);
            None
        });
    }

    {
        let client = client.clone();
        webrtc.connect("on-ice-candidate", false, move |values| {
            println!("ICE Candidate created");
            let sdp_mline_index = values[1].get::<u32>().expect("Invalid argument");
            let candidate = values[2].get::<String>().expect("Invalid argument");

            send_message(&client, &OutgoingMessage {
                command: "ice_candidate".to_string(),
                recipient: identifier.clone(),
                sdp_type: None,
                sdp: None,
                ice_candidate_index: Some(sdp_mline_index),
                ice_candidate: Some(candidate)
            });
            println!("ICE Candidate sent");
            None
        });
    }

    // Start playing
    pipeline
        .set_state(gst::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");
}

fn remove_webrtc(identifier: String, pipeline: &Box::<gst::Pipeline>) {
    println!("Removing Webrtc node");
    let queue = pipeline.by_name(format!("queue-{}",identifier).as_str()).unwrap();
    let webrtc = pipeline.by_name(format!("webrtc-{}",identifier).as_str()).unwrap();
    pipeline.remove_many(&[&queue, &webrtc]).unwrap();
}

fn on_negotiation_needed(client: &Box<Client>, identifier: String, webrtc: &Box::<gst::Element>) {
    let w = webrtc.clone();
    let client = client.clone();
    let promise = gst::Promise::with_change_func(move |reply| {
        let reply = match reply {
            Ok(Some(reply)) => reply,
            Ok(None) => {
                println!("No offer");
                return;
            }
            Err(_) => {
                println!("Error offer");
                return;
            }
        };
        let offer = match reply.value("offer").map(|offer| {
            println!("Offer created");
            offer 
        }) {
            Ok(o) => o,
            Err(_) => {
                return;
            }
        };
        let desc = offer.get::<gst_webrtc::WebRTCSessionDescription>().unwrap();
        println!("SDP Type: {}", desc.type_().to_str());
        println!("SDP :\n{}", desc.sdp().as_text().unwrap());

        let promise = gst::Promise::with_change_func(move |_| {
            send_message(&client, &OutgoingMessage {
                command: "sdp_offer".to_string(),
                recipient: identifier,
                sdp_type: Some(String::from(desc.type_().to_str())),
                sdp: Some(desc.sdp().as_text().unwrap()),
                ice_candidate_index: None,
                ice_candidate: None
            });
            println!("SDP Offer sent\n");
        });
        w.emit_by_name::<()>("set-local-description", &[&offer, &promise]);
    });
    webrtc.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
}

fn on_sdp_anwer(identifier: String, answer: gst_webrtc::WebRTCSessionDescription, pipeline: &Box<gst::Pipeline>) {
    println!("Setting remote description");
    let pipe = pipeline.clone();
    let ident = identifier.clone();
    let promise = gst::Promise::with_change_func(move |_| {
        println!("Remote description set");
        let promise = gst::Promise::with_change_func(move |reply| {
            println!("Webrtc stats ");
        });
        let webrtc = pipe.by_name(format!("webrtc-{}", ident).as_str()).unwrap();
        webrtc.emit_by_name::<()>("get-stats", &[&None::<gst::Pad>, &promise]);
    });
    let webrtc = pipeline.by_name(format!("webrtc-{}", identifier).as_str()).unwrap();
    webrtc.emit_by_name::<()>("set-remote-description", &[&answer, &promise]);
}

fn on_ice_candidate(
    identifier: String,
    sdp_mline_index: u32,
    candidate: &str,
    pipeline: &Box<gst::Pipeline>) {

    let webrtc = pipeline.by_name(format!("webrtc-{}", identifier).as_str()).unwrap();
    webrtc.emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate]);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde()]
struct IncomingMessage {
    pub command: String,
    pub sender: String,
    pub sdp_type: Option<String>,
    pub sdp: Option<String>,
    pub ice_candidate_index: Option<u32>,
    pub ice_candidate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde()]
struct OutgoingMessage {
    pub command: String,
    pub recipient: String,
    pub sdp_type: Option<String>,
    pub sdp: Option<String>,
    pub ice_candidate_index: Option<u32>,
    pub ice_candidate: Option<String>,
}

fn send_message(client: &Box<Client>, message: &OutgoingMessage) {
    let response = client
        .post(URL_MESSAGES)
        .json(&message)
        .send();

    let _response = match block_on(response) {
        Ok(response) => response,
        Err(error) => panic!("Problem sending the message:\n {:?}", error),
    };
}

fn camera_ping(client: &Box<Client>) {
    send_message(client, &OutgoingMessage {
        command: "camera_ping".to_string(),
        recipient: "".to_string(),
        sdp_type: None,
        sdp: None,
        ice_candidate_index: None,
        ice_candidate: None
    });
    println!("Camera ping sent");
}

#[tokio::main]
async fn main() {
    let pipeline: Box<gst::Pipeline> = prepare();

    let client = Box::new(Client::builder()
        .cookie_store(true)
        .build()
        .unwrap());

    let mut stream = client
        .get(URL_SSE)
        .eventsource()
        .unwrap();

    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                let message: IncomingMessage = serde_json::from_str(&event.data).expect("JSON was not well-formatted");
                print!("\nIncoming message from {}: ", message.sender);
                if message.command == "list_cameras" {
                    println!("Camera list");
                    camera_ping(&client);
                } else if message.command == "call" {
                    println!("Call");
                    add_webrtc(&client, message.sender, &pipeline);
                } else if message.command == "end" {
                    println!("Call ended");
                    remove_webrtc(message.sender, &pipeline);
                } else if message.command == "sdp_answer" {
                    println!("New incoming SDP message:");
                    println!("{:?}", &message.sdp_type);
                    println!("{:?}", &message.sdp);
                    let sdp_type = match message.sdp_type {
                        Some(t) => t,
                        None => {
                            println!("Error: No SDP Type !");
                            return;
                        }
                    };
                    let sdp_type = match sdp_type.as_str() {
                        "offer" => WebRTCSDPType::Offer,
                        "pranswer" => WebRTCSDPType::Pranswer,
                        "answer" => WebRTCSDPType::Answer,
                        "rollback" => WebRTCSDPType::Rollback,
                        _ => {
                            println!("Error: Unknown SDP Type !");
                            return;
                        }
                    };

                    let sdp = match SDPMessage::parse_buffer(&message.sdp.unwrap().as_bytes()) {
                        Ok(r) => r,
                        Err(err) => { 
                            println!("Error: Can't parse SDP Description !");
                            println!("{}", err);
                            return;
                        }
                    };
                    on_sdp_anwer(message.sender, WebRTCSessionDescription::new(sdp_type, sdp), &pipeline);
                } else if message.command == "ice_candidate" {
                    println!("New incoming ICE candidate:");
                    println!("{:?}", &message.ice_candidate);
                    let index = match message.ice_candidate_index {
                        Some(i) => i,
                        None => return
                    };
                    let candidate = match &message.ice_candidate {
                        Some(c) => c,
                        None => return
                    };
                    on_ice_candidate(message.sender, index, candidate.as_str(), &pipeline);
                } else {
                    println!("Unknown command :\n{:?}", message);
                }
            },
            Err(error) => {
                println!("Error: {:?}", error);
            }
        }
    }

    // Shutdown pipeline
    pipeline
        .set_state(gst::State::Null)
        .expect("Unable to set the pipeline to the `Null` state");
}
