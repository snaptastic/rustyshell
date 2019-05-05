extern crate rustyline;
extern crate mio;
extern crate flate2;
extern crate getopts;

use mio::*;
use mio::net::*;
use std::io::prelude::*;
use std::process::{exit};
use std::env;

use rustyline::error::ReadlineError;
use rustyline::config::Builder;
use rustyline::Editor;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use flate2::bufread::ZlibDecoder;
use getopts::Options;

const SERVER_TOKEN: Token = Token(0);
const CONNECTION_TOKEN: Token = Token(1);

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [options]", program);
    print!("{}", opts.usage(&brief));
}

fn parse_ip_address(stream: &TcpStream) -> String {
    let cmd_prompt = match stream.peer_addr() {
        Ok(r) => r,
        Err(e) => {
            if e.raw_os_error().unwrap() == 107 {
                println!("[-] Unable to connect to IP address and port.");    
                exit(107);
            }
            println!("[-] Error Parsing IP address. {:?}", e);
            exit(2);
        }
    };
    println!("{:?}", cmd_prompt);
    let cmd_prompt_string = cmd_prompt.ip().to_string();
        
    cmd_prompt_string
}

fn compress_buffer(command_string: &String) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    let _compressed_bytes_len = e.write_all(command_string.as_bytes()).unwrap();
    let compressed_bytes = e.finish().unwrap();
    println!("Compressed len: {}", compressed_bytes.len());
    println!("Orig len: {}", command_string.len());

    compressed_bytes
}

fn command_loop (mut stream: &TcpStream, poll: &Poll, mut pevents: &mut Events) {
    let rustyline_config_builder = Builder::new().max_history_size(1024).auto_add_history(true);
    let rustyline_config = rustyline_config_builder.build();
    let mut readline_editor = Editor::<()>::with_config(rustyline_config);

    if let Err(_) = readline_editor.load_history("history.txt") {
        println!("No previous history.");
    }

    let mut cmd_prompt_string = parse_ip_address(&stream);
    cmd_prompt_string.push_str("> ");

    loop {
        let readline = readline_editor.readline(&cmd_prompt_string);
        match readline {
            Ok(mut line) => {
                line.push_str("\n");

                let command_to_send = compress_buffer(&line);
                println!("Command to send: {:?}", command_to_send);
                let _ = stream.write(&command_to_send);
                if line.trim() == "quit" || line.trim() == "kill" {
                    println!("[+] Exiting....");
                    break;
                }
                if line.trim().len() == 0 {
                    continue;
                }

                poll.poll(&mut pevents, None).unwrap();
                loop {
                    let mut buf = [0; 4096];
                    let recv = match stream.read(&mut buf) {
                        Ok(r) => r,
                        Err(e) => {
                            println!("Error: {}", e);
                            if e.raw_os_error().unwrap() == 11 {
                                break;
                            }
                            break;
                        }
                    };

                    let mut str_buf_clone = buf.to_vec();
                    str_buf_clone.truncate(recv);

                    let mut d = ZlibDecoder::new(str_buf_clone.as_slice());
                    let mut s = String::new();
                    d.read_to_string(&mut s).unwrap();
                    
                    let str_len = s.len() as f32;
                    println!("Results ({:.2}% compression, {} compressed to {}):\n{}", (recv as f32 / str_len) * 100.0 as f32, recv, str_len, s);
                    if recv < 4096 {
                        break;
                    }
                }
            },
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break
            },
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break
            },
            Err(err) => {
                println!("Error: {:?}", err);
                break
            }
        }
    }
}

fn main() {
    let poll = Poll::new().unwrap();
    let mut pevents = Events::with_capacity(2);
    let mut ip_address = String::from("0.0.0.0");
    let mut port = String::from("12345");
    let mut listen = false;

    //Start Arg parsing
    let mut opts = Options::new();
    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();
    opts.optopt("p", "port", "set the port to listen on or connect to", "PORT");
    opts.optopt("i", "ip_address", "set the IP address to connect to", "IP ADDRESS");
    opts.optflag("l", "listen", "set the client to listen on");
    opts.optflag("h", "help", "print this help menu");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => { m }
        Err(f) => { panic!(f.to_string()) }
    };

    if matches.opt_present("h"){
        print_usage(&program, opts);
        return;
    }

    if matches.opt_present("i") && matches.opt_present("l") {
        println!("Can't listen and connect to a remote IP address");
        exit(2);
    }

    if matches.opt_present("l") {
        listen = true;
    }

    if matches.opt_present("p") {
        port = matches.opt_str("p").unwrap();
        println!("{:?}", port);
    }

    if matches.opt_present("i") {
        ip_address = matches.opt_str("i").unwrap();
        println!("{:?}", ip_address);
    }
    //End Arg parsing

    let addr = (ip_address + ":" + &port).parse().unwrap();

    if listen {
        // Create the listener
        let listener = match TcpListener::bind(&addr) {
            Ok(r) => r,
            Err(e) => {
                if e.raw_os_error().unwrap() == 98 {
                    println!("[-] Port already bound");
                    exit(98);
                }
                println!("Error: {:?}", e);
                exit(2);
            }
        };

        poll.register(&listener, SERVER_TOKEN, Ready::readable(), PollOpt::edge()).unwrap();
        let _ = poll.poll(&mut pevents, None);

		let client = listener.accept().unwrap();

        let mut stream = client.0.try_clone().unwrap();
        println!("Accepted connection from {:?}", stream);
        let _ = poll.register(&stream, CONNECTION_TOKEN, Ready::readable(), PollOpt::level());

        command_loop(&mut stream, &poll, &mut pevents);

    } else {
        let mut stream = match TcpStream::connect(&addr) {
            Ok(n) => n,
            Err(err) => { 
                println!("[-] Failed to connect {}", err);
                exit(1);
            },
        };
        let _ = poll.register(&stream, CONNECTION_TOKEN, Ready::readable(), PollOpt::level());
        command_loop(&mut stream, &poll, &mut pevents);
    }
}
