use clap::Parser;
use libp2p::futures::StreamExt;
use libp2p::{identity::Keypair, multiaddr::Protocol, Multiaddr, SwarmBuilder};
use libp2p::{noise, tcp, yamux};
use libp2p_stream as stream;
use std::{error::Error, time::Duration};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use whoami::{on_connection, Request};

#[derive(Parser, Debug)]
#[command(version, about = "Libp2p Whoami Client", long_about = None)]
struct Args {
    /// Multiaddr to connect to
    #[arg(short, long)]
    address: String,
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

    // address to the node I want to call
    let addr: Multiaddr = args.address.parse()?;

    // public/private key of my node.
    let keypair = Keypair::generate_ed25519();
    let mut client = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::new(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_behaviour(|_| stream::Behaviour::new())?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(10)))
        .build();

    client.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;
    client.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    let Some(Protocol::P2p(peer_id)) = addr.iter().last() else {
        panic!("Provided address does not end in /p2p");
    };

    let _ = client.dial(addr)?;

    // Using the peer_id, that we used to dial, we should be expecting
    // a stream to which we must handle to send, and recive our data.
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(on_connection(
        peer_id,
        client.behaviour().new_control(),
        Request {
            user_agent: whoami::UserAgent::Client,
        },
        tx,
    ));

    loop {
        tokio::select! {
            result = rx.recv() => {
                tracing::info!("Finished");
                return Ok(());
            },
            event = client.select_next_some() => {
                match event {
                    libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                        let listen_address = address.with_p2p(*client.local_peer_id()).unwrap();
                        tracing::info!(%listen_address);
                    }
                    event => tracing::trace!(?event),
                }
            }
        }
    }

    Ok(())
}
