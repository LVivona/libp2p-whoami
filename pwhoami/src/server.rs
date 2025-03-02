use clap::Parser;
use std::{error::Error, time::Duration};

use libp2p::{futures::StreamExt, identity::Keypair, noise, tcp, yamux};
use libp2p_stream as stream;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

use whoami::{send_response, Response, WHOAMI_PROTOCOL};

#[derive(Parser, Debug)]
#[command(version, about = "Libp2p Whoami Server", long_about = None)]
struct Args {

    #[arg(short, long)]
    f_name : String,
    
    #[arg(short, long)]
    l_name : Option<String>,

    #[arg(short, long)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env()?,
        )
        .init();

    // ask for key from the user.
    let keypair = Keypair::generate_ed25519();

    let mut server = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| stream::Behaviour::new())?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(10)))
        .build();

    // listening addresses
    server.listen_on(format!("/ip4/0.0.0.0/udp/{}/quic-v1", args.port).parse()?)?;
    server.listen_on(format!("/ip4/0.0.0.0/tcp/{}", args.port).parse()?)?;
    
    let mut incoming_stream = server.behaviour().new_control().accept(WHOAMI_PROTOCOL)?;

    // static response request
    let response = Response::new(
        whoami::UserAgent::Server,
        args.f_name,
        args.l_name
    );
    // handle incoming connection from the the client

    // spawns a event
    tokio::spawn(async move {
        while let Some((peer, stream)) = incoming_stream.next().await {
            match send_response(stream, &response).await {
                Ok(_) => {
                    tracing::info!(%peer, "Reponse sent");
                }
                Err(e) => {
                    println!("Echo failed {e}");
                    tracing::warn!(%peer, "Echo failed: {e}");
                    continue;
                }
            }
        }
    });

    loop {
        let event = server.next().await.expect("Server never terminates");

        match event {
            libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                let addr = address.with_p2p(*server.local_peer_id()).unwrap();
                tracing::info!(%addr);
            }

            event => tracing::trace!(?event),
        }
    }
}
