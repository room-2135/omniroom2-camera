use futures::executor::block_on;
use futures::StreamExt;

use clap::Parser;

use gst::prelude::*;
use gst_sdp::sdp_message::SDPMessage;
use gst_webrtc::{WebRTCSDPType, WebRTCSessionDescription};

mod services;
use services::*;

fn prepare_pipeline(
    video_base: String,
    audio_base: String,
) -> Result<Box<gst::Pipeline>, gst::glib::Error> {
    // Initialize gstreamer
    if let Err(e) = gst::init() {
        eprintln!("Could not initialize gstreamer");
        return Err(e);
    }

    // Build the pipeline
    let pipeline = match gst::parse_launch(&format!(
        "{} ! tee name=videotee ! queue ! fakesink {} ! tee name=audiotee ! queue ! fakesink",
        video_base, audio_base
    )) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to parse initial pipeline");
            return Err(e);
        }
    };

    Ok(Box::new(
        pipeline
            .dynamic_cast::<gst::Pipeline>()
            .expect("Failed to cast into pipeline"),
    ))
}

fn add_webrtc(services: &Box<Services>, identifier: &String, pipeline: &Box<gst::Pipeline>) {
    println!("Adding Webrtc node");

    if let Some(_webrtc) = pipeline.by_name(format!("webrtc-{}", identifier).as_str()) {
        let identifier = identifier.clone();
        remove_webrtc(identifier, pipeline);
    }

    let video_tee = pipeline.by_name("videotee").expect("Cant find videotee");
    let audio_tee = pipeline.by_name("audiotee").expect("Cant find audiotee");
    let video_queue = gst::ElementFactory::make(
        "queue",
        Some(format!("video_queue-{}", identifier).as_str()),
    )
    .expect("Cant find video queue");
    let audio_queue = gst::ElementFactory::make(
        "queue",
        Some(format!("audio_queue-{}", identifier).as_str()),
    )
    .expect("Cant find audio queue");
    let webrtc = Box::new(
        gst::ElementFactory::make("webrtcbin", Some(format!("webrtc-{}", identifier).as_str()))
            .expect("Cant find webrtc node"),
    );

    pipeline
        .add_many(&[&video_queue, &audio_queue, &webrtc])
        .expect("Cant add nodes to pipeline");
    {
        let sinkpad = webrtc
            .request_pad_simple("sink_%u")
            .expect("Cant find sink");
        let srcpad = video_queue.static_pad("src").expect("Cant find src");
        srcpad.link(&sinkpad).expect("Cant link src to sink");
    }
    {
        let sinkpad = webrtc
            .request_pad_simple("sink_%u")
            .expect("Cant find sink");
        let srcpad = audio_queue.static_pad("src").expect("Cant find src");
        srcpad.link(&sinkpad).expect("Cant link src to sink");
    }
    {
        let sinkpad = video_queue.static_pad("sink").expect("Cant find sink");
        let srcpad = video_tee
            .request_pad_simple("src_%u")
            .expect("Cant find src");
        srcpad.link(&sinkpad).expect("Cant link src to sink");
    }
    {
        let sinkpad = audio_queue.static_pad("sink").expect("Cant find sink");
        let srcpad = audio_tee
            .request_pad_simple("src_%u")
            .expect("Cant find src");
        srcpad.link(&sinkpad).expect("Cant link src to sink");
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

            println!("ICE Candidate sending...");
            let services = services.clone();
            let identifier = identifier.clone();
            println!("ICE Candidate about to be sent !");
            block_on(services.send_ice_candidate(&identifier, sdp_mline_index, candidate)).unwrap();
            println!("ICE Candidate sent");
            None
        });
    }

    // Start playing
    pipeline
        .set_state(gst::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");
}

fn remove_webrtc(identifier: String, pipeline: &Box<gst::Pipeline>) {
    println!("Removing Webrtc node");
    let video_queue = pipeline
        .by_name(format!("video_queue-{}", identifier).as_str())
        .expect("Cant find videoqueue");
    let audio_queue = pipeline
        .by_name(format!("audio_queue-{}", identifier).as_str())
        .expect("Cant find audioqueue");
    let webrtc = pipeline
        .by_name(format!("webrtc-{}", identifier).as_str())
        .expect("Cant find webrtc node");
    pipeline
        .remove_many(&[&video_queue, &audio_queue, &webrtc])
        .expect("Cant remove nodes from the pipeline");
}

fn on_negotiation_needed(services: &Box<Services>, identifier: String, webrtc: &Box<gst::Element>) {
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
        let desc = offer
            .get::<gst_webrtc::WebRTCSessionDescription>()
            .expect("Cant extract SDP");
        println!("SDP Type: {}", desc.type_().to_str());
        println!("SDP :\n{}", desc.sdp().as_text().unwrap());

        let promise = gst::Promise::with_change_func(move |_| {
            println!("SDP Offer about to be sent !");
            block_on(services.send_sdp_offer(&identifier, desc.sdp().as_text().unwrap())).unwrap();
            println!("SDP Offer sent\n");
        });
        w.emit_by_name::<()>("set-local-description", &[&offer, &promise]);
    });
    //webrtc.set_property("stun-server", "stun:stun.l.google.com:19302");
    //webrtc.set_property("turn-server", "turn://muazkh:webrtc@live.com@numb.viagenie.ca");
    webrtc.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
}

fn handle_sdp_anwer(
    identifier: &String,
    answer: gst_webrtc::WebRTCSessionDescription,
    pipeline: &Box<gst::Pipeline>,
) {
    println!("Setting remote description");
    let promise = gst::Promise::with_change_func(move |_| {
        println!("Remote description set");
    });
    let webrtc = pipeline
        .by_name(format!("webrtc-{}", identifier).as_str())
        .expect("Cant find webrtc node");
    webrtc.emit_by_name::<()>("set-remote-description", &[&answer, &promise]);
}

fn handle_ice_candidate(
    identifier: &String,
    sdp_mline_index: u32,
    candidate: &str,
    pipeline: &Box<gst::Pipeline>,
) {
    let webrtc = pipeline
        .by_name(format!("webrtc-{}", identifier).as_str())
        .expect("Cant find webrtc node");
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
    #[clap(
        long,
        default_value = "videotestsrc ! videoconvert ! queue ! x264enc tune=zerolatency ! video/x-h264,profile=high ! rtph264pay"
    )]
    video_base: String,

    /// Gstreamer audio pipeline base
    #[clap(
        long,
        default_value = "audiotestsrc ! audioconvert ! queue ! opusenc ! rtpopuspay"
    )]
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

    let signaling_protocol = if args.unsecure { "http" } else { "https" };
    let uri = format!("{}://{}:{}", signaling_protocol, args.address, args.port);

    println!("Connecting to signaling server with uri: {}", uri);
    let services = Box::new(Services::new(uri));
    {
        let pipeline = &pipeline.clone();
        let services = &services.clone();

        let mut stream = services.start_sse();
        while let Some(event) = stream.next().await {
            match event {
                Ok(event) => match event {
                    reqwest_eventsource::Event::Message(m) => {
                        let message: IncomingMessage = serde_json::from_str(&m.data)
                            .expect("Failed to parse received command");
                        match message.payload {
                            Payload::Welcome { .. } => {
                                println!("Welcomed");
                                services
                                    .send_new_camera()
                                    .await
                                    .expect("Failed to send new camera event");
                            }
                            Payload::CameraDiscovery => {
                                services
                                    .send_camera_ping(&message.sender)
                                    .await
                                    .expect("Failed to send a camera ping");
                                println!("Camera ping sent");
                            }
                            Payload::CallInit => {
                                println!("Call");
                                add_webrtc(&services, &message.sender, &pipeline);
                            }
                            Payload::SDP { .. } => {
                                println!("New incoming SDP message:");
                                if let Payload::SDP { description } = &message.payload {
                                    println!("{:?}", description);
                                    let sdp = match SDPMessage::parse_buffer(description.as_bytes())
                                    {
                                        Ok(r) => r,
                                        Err(err) => {
                                            println!("Error: Can't parse SDP Description !");
                                            println!("{:?}", err);
                                            return;
                                        }
                                    };
                                    handle_sdp_anwer(
                                        &message.sender,
                                        WebRTCSessionDescription::new(WebRTCSDPType::Answer, sdp),
                                        &pipeline,
                                    );
                                }
                            }
                            Payload::ICE { .. } => {
                                println!("New incoming ICE candidate:");
                                if let Payload::ICE { index, candidate } = &message.payload {
                                    println!("{:?}", candidate);
                                    handle_ice_candidate(
                                        &message.sender,
                                        *index,
                                        candidate.as_str(),
                                        &pipeline,
                                    );
                                };
                            }
                            _ => {
                                eprintln!(
                                "Error: Camera is no supposed to receive this payload type: {:?}",
                                message.payload
                            );
                            }
                        }
                    }
                    _ => {}
                },
                Err(error) => {
                    println!("=============");
                    println!("Error: {:?}", error);
                }
            }
        }
    }

    // Shutdown pipeline
    pipeline
        .set_state(gst::State::Null)
        .expect("Unable to set the pipeline to the `Null` state");
}
