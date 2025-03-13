use clap::Parser;
use libp2p::futures::StreamExt;
use libp2p::{identity::Keypair, multiaddr::Protocol, Multiaddr, SwarmBuilder};
use libp2p::{noise, tcp, yamux};
use libp2p_stream as stream;
use std::io::Write;
use std::{error::Error, time::Duration};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;
use whoami::{on_connection, Request, Response};

#[derive(Parser, Debug)]
#[command(version, about = "Libp2p Whoami Client", long_about = None)]
struct Args {
    /// Multiaddr to connect to
    #[arg(short, long)]
    address: String,

    #[arg(short, long, default_value = "false")]
    pretty_print : bool
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

    // address to the node I want `[Ne]`
    let addr: Multiaddr = match args.address.parse() {
        Ok(addr) => addr,
        Err(_) => {
            // Fall back to custom parsing with the whoami module
            let address_str = args.address;
            whoami::from_url("whoami", &address_str, false, 4040)?
        }
    };
    // public/private key of my node.
    let keypair = Keypair::generate_ed25519();
    let mut client = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()               // tokio runtime enviorment
        .with_tcp(                  // tcp over ip network communication layer
            tcp::Config::new(),
            noise::Config::new, 
            yamux::Config::default,
        )?
        .with_quic()                // quic, udp over ip network communciation layer
        .with_dns()?
        .with_behaviour(|_| stream::Behaviour::new())?  // bytes stream behavour
        .with_swarm_config(|c| 
            c.with_idle_connection_timeout(Duration::from_secs(10)))
        .build();

    client.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;
    client.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    let Some(Protocol::P2p(peer_id)) = addr.iter().last() else {
        panic!("Provided address does not end in /p2p");
    };

    let _ = client.dial(addr)?;

    // Using the peer_id, that we used to dial, we should be expecting
    // a stream to which we must handle to send, and recive our data.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Response>(1024);
    tokio::spawn(on_connection(
        peer_id,
        client.behaviour().new_control(),
        Request::default(),
        tx,
    ));

    loop {
        tokio::select! {
            Some(res) = rx.recv() => {
                tracing::info!("Finished");
                if args.pretty_print {
                    let mut stdout = std::io::stdout().lock();
                    let _ = stdout.write(&format!("{:=^50}\n\n", " [whoami] ").into_bytes())?;
                    let _ = stdout.write_fmt(format_args!("First Name:{:^10}\n", res.f_name))?;
                    if let Some(l_name) = res.l_name {
                        let _ = stdout.write_fmt(format_args!("Last Name:{:^10}\n\n", l_name))?;
                    }
                    let _ = stdout.write(&format!("{:=<50}\n", "").into_bytes())?;
                } else {
                    println!("{:?}", res);
                }
                
                return Ok(());
            },
            event = client.select_next_some() => {
                match event {
                    
                    libp2p::swarm::SwarmEvent::NewListenAddr {address, .. } => {
                        let listen_address = address.with_p2p(*client.local_peer_id()).unwrap();
                        tracing::info!(%listen_address);
                    },  
                    event => tracing::trace!(?event),
                }
            }
        }
    }

}
