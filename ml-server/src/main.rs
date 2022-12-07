use std::{
    collections::HashSet,
    convert::Infallible,
    env, fs,
    net::{IpAddr, SocketAddr},
    process::Stdio,
    sync::Arc,
    time::Instant,
};

use hyper::{server::conn::AddrStream, service, Body, Request, Response, Server};
use serde::Deserialize;
use tokio::{
    io::{stdin, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::Mutex,
};

lazy_static::lazy_static! {
    static ref ACCEPTED_ADDRESSES: Arc<Mutex<HashSet<IpAddr>>> = Arc::new(Mutex::new(HashSet::new()));
    static ref START_TIME: Arc<Instant> = Arc::new(Instant::now());
    static ref CONFIG: Arc<Config> = Arc::new(ron::de::from_str(&fs::read_to_string(
        env::var("CONFIG").unwrap_or("./server-config.ron".to_string()),
    ).unwrap()).unwrap());
}

macro_rules! log {
    ($($arg:expr),*) => {
        println!("\x1b[31m[{:.2}]\x1b[0m \x1b[32m==\x1b[0m {}", START_TIME.elapsed().as_secs_f64(), format!($($arg,)*))
    }
}

#[derive(Deserialize)]
struct Config {
    apikey: String,
    java: String,
    port: u16,
    working_dir: String,
    args: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Triggering the lazy static macro so the objects get created
    let _ = START_TIME.elapsed();

    let auth_addr = SocketAddr::from(([0, 0, 0, 0], CONFIG.port));

    let auth_server =
        Server::bind(&auth_addr).serve(service::make_service_fn(move |socket: &AddrStream| {
            let addr = socket.remote_addr();
            async move { Ok::<_, Infallible>(service::service_fn(move |req| auth(req, addr))) }
        }));

    log!("listening on {}", auth_addr.ip());

    tokio::select! {
        ret = auth_server => if let Err(why) = ret {
            log!("auth server error: {}", why)
        },
        _ = mc_server() => (),
    }

    log!("Server exit");

    std::process::exit(0);
}

async fn mc_server() -> Result<(), Box<dyn std::error::Error>> {
    let mut handle = Command::new(&CONFIG.java)
        .args(&CONFIG.args)
        .current_dir(&CONFIG.working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let main_stdin = Arc::new(Mutex::new(handle.stdin.take().unwrap()));
    let control_stdin = main_stdin.clone();
    let stdout = handle.stdout.take().unwrap();

    tokio::select! {
        _ = handle.wait() => (),
        _ = async move {
            let mut lines = BufReader::new(stdin()).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                main_stdin.lock().await.write_all(format!("{}\n", line).as_bytes()).await.unwrap();
            }
        } => (),
        _ = async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("logged in") {
                    if let Err(why) = handle_login(&line, control_stdin.clone()).await {
                        log!("error handling player login: {}", why);
                    }
                }
                println!("\x1b[90m{}\x1b[0m", line);
            }
        } => (),
    }
    Ok(())
}

async fn handle_login(
    line: &String,
    control_stdin: Arc<Mutex<ChildStdin>>,
) -> Result<(), Box<dyn std::error::Error>> {
    log!("user connecting!");
    let message = match line.splitn(4, ":").skip(3).next() {
        Some(message) => message,
        None => return Err("malformed log line".to_string().into()),
    };
    let (username, _) = match message.split_once("[") {
        Some(message) => message,
        None => return Err("malformed log line".to_string().into()),
    };
    let username = username.trim();
    let start = message.find("[").unwrap_or(0) + 1;
    let end = message.find("]").unwrap_or(line.len());

    let addr_slice = &message[start..end];

    let (_, addr) = match addr_slice.split_once("/") {
        Some(message) => message,
        None => return Err("malformed log line".to_string().into()),
    };

    let addr = addr.parse::<SocketAddr>()?;

    log!("address: {}", addr.ip());

    if !ACCEPTED_ADDRESSES.lock().await.contains(&addr.ip()) {
        log!("address not authenticated! kicking {}!", username);
        control_stdin
            .lock()
            .await
            .write_all(format!("/kick {}\n", username).as_bytes())
            .await?;
    }
    Ok(())
}

async fn auth(req: Request<Body>, addr: SocketAddr) -> Result<Response<Body>, hyper::Error> {
    let addr = match req.headers().get("X-Real-IP") {
        Some(ip) => {
            log!(
                "X-Real-IP header received: {}",
                ip.to_str().unwrap_or_default()
            );
            ip.to_str()
                .unwrap_or_default()
                .parse::<IpAddr>()
                .unwrap_or("127.0.0.1".parse::<IpAddr>().unwrap())
        }
        None => addr.ip(),
    };

    log!("{} requested auth", addr);
    match req.headers().get("APIKey") {
        Some(key) => {
            if key.to_str().unwrap_or("") == CONFIG.apikey {
                ACCEPTED_ADDRESSES.lock().await.insert(addr);
                log!("{} auth accepted", addr);
                Ok(Response::builder().status(200).body(Body::empty()).unwrap())
            } else {
                log!("{} wrong APIKey", addr);
                Ok(Response::builder().status(401).body(Body::empty()).unwrap())
            }
        }
        None => {
            log!("{} no APIKey provided", addr);
            Ok(Response::builder().status(401).body(Body::empty()).unwrap())
        }
    }
}
