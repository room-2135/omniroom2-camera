use reqwest::blocking::Client;
use sse_client::EventSource;
use serde::{Serialize, Deserialize};


use gst::prelude::*;
use gst_webrtc::{WebRTCSessionDescription, WebRTCSDPType};
use gst_sdp::sdp_message::SDPMessage;

const URL_SSE: &str = "http://localhost:8000/events";
const URL_MESSAGES: &str = "http://localhost:8000/message";

fn prepare() -> Box<gst::Pipeline> {
    // Initialize gstreamer
    gst::init().unwrap();

    // Build the pipeline
    let pipeline = gst::parse_launch(&format!("v4l2src device=/dev/video1 ! videoconvert ! videoscale ! video/x-raw,width=1920,height=1080,framerate=30/1 ! x264enc tune=zerolatency ! video/x-h264,profile=baseline ! rtph264pay ! application/x-rtp,media=video,encoding-name=H264,payload=96 ! tee name=videotee ! queue ! fakesink")).unwrap();
    Box::new(pipeline.dynamic_cast::<gst::Pipeline>().unwrap())
}

fn add_webrtc(pipeline: &Box::<gst::Pipeline>) {
    println!("Adding Webrtc node");
    let queue = gst::ElementFactory::make("queue", Some("queue-test")).expect("");
    let webrtc = Box::new(gst::ElementFactory::make("webrtcbin", Some("webrtc-test")).expect(""));

    pipeline.add_many(&[&queue, &webrtc]).unwrap();

    let srcpad = queue.static_pad("src").unwrap();
    let sinkpad = webrtc.request_pad_simple("sink_%u").unwrap();
    srcpad.link(&sinkpad).unwrap();

    let tee = pipeline.by_name("videotee").unwrap();
    let srcpad = tee.request_pad_simple("src_%u").unwrap();
    let sinkpad = queue.static_pad("sink").unwrap();
    srcpad.link(&sinkpad).unwrap();

    let w = webrtc.clone();
    webrtc.connect("on-negotiation-needed", false, move |_| {
        println!("Negociation needed");
        on_negotiation_needed(&w);
        None
    });

    webrtc.connect("on-ice-candidate", false, move |values| {
        println!("ICE Candidate created");
        let sdp_mline_index = values[1].get::<u32>().expect("Invalid argument");
        let candidate = values[2].get::<String>().expect("Invalid argument");

        send_message(&Message {
            command: "ice_candidate".to_string(),
            identifier: "camera1".to_string(),
            sdp_type: None,
            sdp: None,
            ice_candidate_index: Some(sdp_mline_index),
            ice_candidate: Some(candidate)
        });
        println!("ICE Candidate sent");
        None
    });

    // Start playing
    pipeline
        .set_state(gst::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");
}

fn remove_webrtc(pipeline: &Box::<gst::Pipeline>) {
    println!("Removing Webrtc node");
    let queue = pipeline.by_name("queue-test").unwrap();
    let webrtc = pipeline.by_name("webrtc-test").unwrap();
    pipeline.remove_many(&[&queue, &webrtc]).unwrap();
}

fn run(pipeline: Box<gst::Pipeline>) {
    // Wait until error or EOS
    let bus = pipeline.bus().unwrap();
    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        use gst::MessageView;

        match msg.view() {
            MessageView::Eos(..) => break,
            MessageView::Error(err) => {
                println!(
                    "Error from {:?}: {} ({:?})",
                    err.src().map(|s| s.path_string()),
                    err.error(),
                    err.debug()
                );
                break;
            }
            _ => (),
        }
    }

    // Shutdown pipeline
    pipeline
        .set_state(gst::State::Null)
        .expect("Unable to set the pipeline to the `Null` state");
}

fn on_negotiation_needed(webrtc: &Box::<gst::Element>) {
    let w = webrtc.clone();
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
            send_message(&Message {
                command: "sdp_offer".to_string(),
                identifier: "camera1".to_string(),
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

fn on_sdp_anwer(answer: gst_webrtc::WebRTCSessionDescription, pipeline: &Box<gst::Pipeline>) {

    println!("Setting remote description");
    let pipe = pipeline.clone();
    let promise = gst::Promise::with_change_func(move |_| {
        println!("Remote description set");
        let promise = gst::Promise::with_change_func(move |reply| {
            println!("Webrtc stats ");
        });
        let webrtc = pipe.by_name("webrtc-test").unwrap();
        webrtc.emit_by_name::<()>("get-stats", &[&None::<gst::Pad>, &promise]);
    });
    let webrtc = pipeline.by_name("webrtc-test").unwrap();
    webrtc.emit_by_name::<()>("set-remote-description", &[&answer, &promise]);
}

fn on_ice_candidate(
    sdp_mline_index: u32,
    candidate: &str,
    pipeline: &Box<gst::Pipeline>) {

    let webrtc = pipeline.by_name("webrtc-test").unwrap();
    webrtc.emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate]);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde()]
struct Message {
    pub command: String,
    pub identifier: String,
    pub sdp_type: Option<String>,
    pub sdp: Option<String>,
    pub ice_candidate_index: Option<u32>,
    pub ice_candidate: Option<String>
}

fn send_message(message: &Message) {
    let response = Client::new()
        .post(URL_MESSAGES)
        .json(&message)
        .send();

    let _response = match response {
        Ok(response) => response,
        Err(error) => panic!("Problem sending the message:\n {:?}", error),
    };
}

fn camera_ping() {
    send_message(&Message {
        command: "camera_ping".to_string(),
        identifier: "camera1".to_string(),
        sdp_type: None,
        sdp: None,
        ice_candidate_index: None,
        ice_candidate: None
    });
    println!("Camera ping sent");
}

fn main() {
    let pipeline: Box<gst::Pipeline> = prepare();

    let event_source = EventSource::new(URL_SSE).unwrap();

    event_source.on_open(|| {
        println!("Successfully connected !");
        camera_ping();
    });

    {
        let pipe = pipeline.clone();
        event_source.on_message(move |message_raw| {
            println!("{}", message_raw.data);
            let message: Message = serde_json::from_str(&message_raw.data).expect("JSON was not well-formatted");
            if message.identifier == "camera1" {
                return;
            }
            print!("\nIncoming message: ");
            if message.command == "list_cameras" {
                println!("Camera list");
                camera_ping();
            } else if message.command == "call" {
                println!("Call");
                add_webrtc(&pipe);
            } else if message.command == "end" {
                println!("Call ended");
                remove_webrtc(&pipe);
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
                on_sdp_anwer(WebRTCSessionDescription::new(sdp_type, sdp), &pipe);
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
                on_ice_candidate(index, candidate.as_str(), &pipe);
            } else {
                println!("Unknown command :\n{:?}", message);
            }
        });
    }

    event_source.add_event_listener("error", |error| {
        println!("Error: {:?}", error);
    });

    run(pipeline);
}
