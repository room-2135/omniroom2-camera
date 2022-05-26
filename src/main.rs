use gst::prelude::*;
use gst_webrtc::{WebRTCSessionDescription, WebRTCSDPType};
use gst_sdp::sdp_message::SDPMessage;

mod services;
use services::*;

use clap::Parser;

fn prepare_pipeline(video_base: String, audio_base: String) -> Result<Box<gst::Pipeline>, gst::glib::Error> {
    // Initialize gstreamer
    if let Err(e) = gst::init() {
        eprintln!("Could not initialize gstreamer");
        return Err(e);
    }

    // Build the pipeline
    let pipeline = match gst::parse_launch(&format!("{} ! tee name=videotee ! queue ! fakesink {} ! tee name=audiotee ! queue ! fakesink", video_base, audio_base)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to parse initial pipeline");
            return Err(e);
        }
    };

    Ok(Box::new(pipeline.dynamic_cast::<gst::Pipeline>().unwrap()))
}

fn add_webrtc(services: &Box<Services>, identifier: &String,  pipeline: &Box::<gst::Pipeline>) {
    println!("Adding Webrtc node");

    if let Some(_webrtc) = pipeline.by_name(format!("webrtc-{}", identifier).as_str()) {
        let identifier = identifier.clone();
        remove_webrtc(identifier, pipeline);
    }

    let video_tee = pipeline.by_name("videotee").unwrap();
    let audio_tee = pipeline.by_name("audiotee").unwrap();
    let video_queue =  gst::ElementFactory::make("queue", Some(format!("video_queue-{}", identifier).as_str())).expect("");
    let audio_queue =  gst::ElementFactory::make("queue", Some(format!("audio_queue-{}", identifier).as_str())).expect("");
    let webrtc = Box::new(gst::ElementFactory::make("webrtcbin", Some(format!("webrtc-{}", identifier).as_str())).expect(""));

    pipeline.add_many(&[&video_queue, &audio_queue, &webrtc]).unwrap();
    {
        let sinkpad = webrtc.request_pad_simple("sink_%u").unwrap();
        let srcpad = video_queue.static_pad("src").unwrap();
        srcpad.link(&sinkpad).unwrap();
    }
    {
        let sinkpad = webrtc.request_pad_simple("sink_%u").unwrap();
        let srcpad = audio_queue.static_pad("src").unwrap();
        srcpad.link(&sinkpad).unwrap();
    }
    {
        let sinkpad = video_queue.static_pad("sink").unwrap();
        let srcpad = video_tee.request_pad_simple("src_%u").unwrap();
        srcpad.link(&sinkpad).unwrap();
    }
    {
        let sinkpad = audio_queue.static_pad("sink").unwrap();
        let srcpad = audio_tee.request_pad_simple("src_%u").unwrap();
        srcpad.link(&sinkpad).unwrap();
    }

    {
        let w = webrtc.clone();
        let services = services.clone();
        let identifier = identifier.clone();
        webrtc.connect("on-negotiation-needed", false, move |_| {
            println!("Negociation needed");
            on_negotiation_needed(&services, identifier.clone(), &w);
            None
        });
    }

    {
        let services = services.clone();
        let identifier = identifier.clone();
        webrtc.connect("on-ice-candidate", false, move |values| {
            println!("ICE Candidate created");
            let sdp_mline_index = values[1].get::<u32>().expect("Invalid argument");
            let candidate = values[2].get::<String>().expect("Invalid argument");

            services.send_ice_candidate(&identifier, sdp_mline_index, candidate);
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
    let video_queue = pipeline.by_name(format!("video_queue-{}",identifier).as_str()).unwrap();
    let audio_queue = pipeline.by_name(format!("audio_queue-{}",identifier).as_str()).unwrap();
    let webrtc = pipeline.by_name(format!("webrtc-{}",identifier).as_str()).unwrap();
    pipeline.remove_many(&[&video_queue, &audio_queue, &webrtc]).unwrap();
}

fn on_negotiation_needed(services: &Box<Services>, identifier: String, webrtc: &Box::<gst::Element>) {
    let w = webrtc.clone();
    let services = services.clone();
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
            services.send_sdp_offer(&identifier, desc.sdp().as_text().unwrap());
            println!("SDP Offer sent\n");
        });
        w.emit_by_name::<()>("set-local-description", &[&offer, &promise]);
    });
    //webrtc.set_property("stun-server", "stun:stun.l.google.com:19302");
    //webrtc.set_property("turn-server", "turn://muazkh:webrtc@live.com@numb.viagenie.ca");
    webrtc.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
}

fn handle_sdp_anwer(identifier: &String, answer: gst_webrtc::WebRTCSessionDescription, pipeline: &Box<gst::Pipeline>) {
    println!("Setting remote description");
    let promise = gst::Promise::with_change_func(move |_| {
        println!("Remote description set");
    });
    let webrtc = pipeline.by_name(format!("webrtc-{}", identifier).as_str()).unwrap();
    webrtc.emit_by_name::<()>("set-remote-description", &[&answer, &promise]);
}

fn handle_ice_candidate(identifier: &String, sdp_mline_index: u32, candidate: &str, pipeline: &Box<gst::Pipeline>) {
    let webrtc = pipeline.by_name(format!("webrtc-{}", identifier).as_str()).unwrap();
    webrtc.emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate]);
}

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Signaling server address
    #[clap(long, default_value = "localhost")]
    address: String,

    /// Signaling server port
    #[clap(long, default_value = "8000")]
    port: u32,

    /// Use SSL/TLS to contact Signaling server
    #[clap(long)]
    unsecure: bool,

    /// Gstreamer video pipeline base
    #[clap(long, default_value = "videotestsrc ! videoconvert ! queue ! x264enc tune=zerolatency ! video/x-h264,profile=high ! rtph264pay")]
    video_base: String,

    /// Gstreamer audio pipeline base
    #[clap(long, default_value = "audiotestsrc ! audioconvert ! queue ! opusenc ! rtpopuspay")]
    audio_base: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let pipeline = match prepare_pipeline(args.video_base, args.audio_base) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to initialize pipeline:\n{:?}", e);
            return;
        }
    };

    let signaling_protocol = if args.unsecure {"http"} else {"https"};
    let uri = format!("{}://{}:{}/", signaling_protocol, args.address, args.port);

    println!("Connecting to signaling server with uri: {}", uri);
    let services = Box::new(Services::new(uri));
    {
        let pipeline = &pipeline.clone();
        let services = &services.clone();
        services.start_sse(move |message: &IncomingMessage| {
            services.send_camera_ping(&message.sender);
            println!("Camera ping sent");
        }, move |message: &IncomingMessage| {
            println!("Call");
            add_webrtc(&services, &message.sender, &pipeline);
        }, move |message: &IncomingMessage| {
            println!("New incoming SDP message:");
            if let Payload::SDP { description } = &message.payload {
                println!("{:?}", description);
                let sdp = match SDPMessage::parse_buffer(description.as_bytes()) {
                    Ok(r) => r,
                    Err(err) => { 
                        println!("Error: Can't parse SDP Description !");
                        println!("{:?}", err);
                        return;
                    }
                };
                handle_sdp_anwer(&message.sender, WebRTCSessionDescription::new(WebRTCSDPType::Answer, sdp), &pipeline);
            }
        }, move |message: &IncomingMessage| {
            println!("New incoming ICE candidate:");
            if let Payload::ICE { index, candidate } = &message.payload {
                println!("{:?}", candidate);
                handle_ice_candidate(&message.sender, *index, candidate.as_str(), &pipeline);
            };
        }).await;
    }

    // Shutdown pipeline
    pipeline
        .set_state(gst::State::Null)
        .expect("Unable to set the pipeline to the `Null` state");
}
