use crossbeam_channel::bounded;
use rosc::encoder;
use rosc::{OscMessage, OscPacket, OscType};
use serde_json::to_string;
use crate::config::Config;
use crate::control::{ControlMessage, SlicerMessage};
use std::net::{SocketAddr, SocketAddrV4, UdpSocket};
use std::str::FromStr;
use std::thread;
use crate::sample_gen::slicer::TransformType;

/// OSCRemoteControl keeps track of the remote controller app that control this smplr instance
struct OSCRemoteControl {
    address: Option<SocketAddr>,
}

/// Port of the remote OSC app
const OSC_REMOTE_CONTROL_PORT: u16 = 6666;

/// Initialize the OSC thread / routines
pub fn initialize_osc(
    conf: Config,
) -> (
    thread::JoinHandle<()>,
    crossbeam_channel::Sender<ControlMessage>,
    crossbeam_channel::Receiver<ControlMessage>,
) {
    // initialise the IN -> OUT crossbeam bus
    let (out_cx_tx, out_cx_rx) = bounded::<ControlMessage>(1024);

    // initialise the OUT -> IN crossbeam bus
    let (in_cx_tx, _in_cx_rx) = bounded::<ControlMessage>(1024);

    // init the osc thread
    let osc_thread = thread::spawn(move || {
        // better name
        let command_tx = out_cx_tx;

        // keep track of the remote UI controller using this datastruct
        let mut osc_controller = OSCRemoteControl { address: None };

        // init host address
        let host_addr = SocketAddrV4::from_str("0.0.0.0:6667").unwrap();

        // init the receiving socket
        let socket = UdpSocket::bind(host_addr).unwrap();
        println!("osc: Listening to {}", host_addr);

        // OSC buffer
        let mut buf = [0u8; rosc::decoder::MTU];

        // OSC loop
        loop {
            match socket.recv_from(&mut buf) {
                Ok((size, addr)) => {
                    // println!("osc: Received packet with size {} from: {}", size, addr);
                    let packet = rosc::decoder::decode(&buf[..size]).unwrap();
                    handle_incoming_packet(
                        packet,
                        addr,
                        &mut osc_controller,
                        &socket,
                        &conf,
                        command_tx.clone(),
                    );
                }
                Err(e) => {
                    println!("osc: Error receiving from socket: {}", e);
                    break;
                }
            }
        }
    });

    // return thread handle and receiver
    return (osc_thread, in_cx_tx, out_cx_rx);
}

// handle an incoming os packet
fn handle_incoming_packet(
    packet: OscPacket,
    from: SocketAddr,
    osc_controller: &mut OSCRemoteControl,
    socket: &UdpSocket,
    conf: &Config,
    command_tx: crossbeam_channel::Sender<ControlMessage>,
) {
    match packet {
        OscPacket::Message(msg) => {
            // route this packet
            match msg.addr.as_str() {
                // ping is important to keep the state of connection
                "/smplr/ping" => handle_ping(from, osc_controller, socket, msg),
                // remote control ui is asking for config toml as serialized string
                "/smplr/get_config" => {
                    // serialize the conf to hson string
                    // can't use toml because datastruct support is too limited
                    let serialized_conf = to_string(conf).unwrap();

                    // creates set_config osc message
                    let msg_buf = encoder::encode(&OscPacket::Message(OscMessage {
                        addr: "/smplr/set_config".to_string(),
                        args: Some(vec![OscType::String(serialized_conf)]),
                    }))
                    .unwrap();

                    // extract addr
                    let send_to = osc_controller.address.unwrap();

                    // send back the config
                    socket.send_to(&msg_buf, send_to).unwrap();
                }
                // track volume
                // @TODO take care of the message timecodes
                "/smplr/track/volume" => {
                    let args = msg.args.unwrap();
                    // nice way to handle args :D
                    match (&args[0], &args[1]) {
                        (OscType::Int(idx), OscType::Float(val)) => {
                            // build message
                            let m = ControlMessage::TrackVolume {
                                tcode: 0,
                                val: *val,
                                track_num: *idx as usize,
                            };
                            // send
                            command_tx.try_send(m).unwrap();
                        }
                        _ => {}
                    }
                }
                "/smplr/track/pan" => {
                    let args = msg.args.unwrap();
                    // nice way to handle args :D
                    match (&args[0], &args[1]) {
                        (OscType::Int(idx), OscType::Float(val)) => {
                            // build message
                            let m = ControlMessage::TrackPan {
                                tcode: 0,
                                val: *val,
                                track_num: *idx as usize,
                            };
                            // send
                            command_tx.try_send(m).unwrap();
                        }
                        _ => {}
                    }
                }
                "/smplr/track/loop_div" => {
                    let args = msg.args.unwrap();
                    // nice way to handle args :D
                    match (&args[0], &args[1]) {
                        (OscType::Int(idx), OscType::Int(val)) => {
                            // build message
                            let m = ControlMessage::TrackLoopDiv {
                                tcode: 0,
                                val: *val as u64,
                                track_num: *idx as usize,
                            };
                            // send
                            command_tx.try_send(m).unwrap();
                        }
                        _ => {}
                    }
                }
                "/smplr/track/next_sample" => {
                    let args = msg.args.unwrap();
                    // nice way to handle args :D
                    match &args[0] {
                        OscType::Int(idx) => {
                            // build message
                            let m = ControlMessage::TrackNextSample {
                                tcode: 0,
                                track_num: *idx as usize,
                            };
                            // send
                            command_tx.try_send(m).unwrap();
                        }
                        _ => {}
                    }
                },
                "/smplr/track/prev_sample" => {
                    let args = msg.args.unwrap();
                    // nice way to handle args :D
                    match &args[0] {
                        OscType::Int(idx) => {
                            // build message
                            let m = ControlMessage::TrackPrevSample {
                                tcode: 0,
                                track_num: *idx as usize,
                            };
                            // send
                            command_tx.try_send(m).unwrap();
                        }
                        _ => {}
                    }
                },
                "/smplr/track/slicer/transform" => {
                    let args = msg.args.unwrap();
                    // nice way to handle args :D
                    match (&args[0], &args[1]) {
                        (OscType::Int(idx), OscType::String(t)) => {
                            match &t[..] {
                                "reset" => {
                                    let _res = command_tx.try_send(ControlMessage::Slicer {
                                        tcode: 0,
                                        track_num: *idx as usize,
                                        message: SlicerMessage::Transform(TransformType::Reset())
                                    });
                                }
                                "rand_swap" => {
                                    let _res = command_tx.try_send(ControlMessage::Slicer {
                                        tcode: 0,
                                        track_num: *idx as usize,
                                        message: SlicerMessage::Transform(TransformType::RandSwap())
                                    });
                                }
                                _ => {}, // unknown
                            };
                        }
                        _ => {}
                    }
                },
                "/smplr/track/slicer/repeat" => {
                    let args = msg.args.unwrap();
                    // nice way to handle args :D
                    match (&args[0], &args[1]) {
                        (OscType::Int(idx), OscType::Int(q)) => {
                            let _res = command_tx.try_send(ControlMessage::Slicer {
                                tcode: 0,
                                track_num: *idx as usize,
                                message: SlicerMessage::Transform(TransformType::QuantRepeat{
                                    quant: *q as usize,
                                    slice_index: 0, // unused here
                                })
                            });
                        }
                        _ => {}
                    }
                }
                _ => {
                    println!("osc: unimplemented adress: {:?}", msg.addr);
                }
            };
        }
        OscPacket::Bundle(_bundle) => {
            // println!("osc: OSC Bundle: {:?}", bundle);
        }
    }
}

// handle ping form controller
fn handle_ping(
    from: SocketAddr,
    osc_controller: &mut OSCRemoteControl,
    socket: &UdpSocket,
    msg: OscMessage,
) {
    match msg.args {
        Some(args) => {
            let rnd_ping = &args[0];
            match rnd_ping {
                OscType::Int(r) => {
                    // init the remote control
                    // change port to expected remote port
                    let mut new_from = from.clone();
                    new_from.set_port(OSC_REMOTE_CONTROL_PORT);
                    if osc_controller.address == None {
                        osc_controller.address = Some(new_from);
                    }

                    // creates pingback osc message
                    let msg_buf = encoder::encode(&OscPacket::Message(OscMessage {
                        addr: "/smplr/ping_back".to_string(),
                        args: Some(vec![OscType::Int(*r)]),
                    }))
                    .unwrap();

                    // extract addr
                    let send_to = osc_controller.address.unwrap();

                    // send back
                    socket.send_to(&msg_buf, send_to).unwrap();
                }
                _ => println!("osc: incorrect type ping, ignoring ..."),
            }
        }
        None => println!("osc: No arguments in ping, ignoring ..."),
    }
}
