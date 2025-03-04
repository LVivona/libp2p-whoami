use clap::Parser;
use std::{error::Error, time::Duration};

use libp2p::{futures::StreamExt, noise, tcp, yamux};
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

    #[arg(short, long)]
    dns : bool
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

    let mut server  = libp2p::SwarmBuilder::with_new_identity() 
        .with_tokio()   // tokio runtime
        .with_tcp(      
            tcp::Config::default(), // tcp Config defautlt
            noise::Config::new,     
            yamux::Config::default,
        )? 
        .with_quic()    // quic, and udp on top of ip4/ip6
        .with_dns()?
        .with_behaviour(|_| stream::Behaviour::new())?  // stream behavioud
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(10))) // duration of stream lasts 10 seconds until given up
        .build();


    // listening addresses
    server.listen_on(format!("/ip4/0.0.0.0/udp/{}/quic-v1", args.port).parse()?)?;
    server.listen_on(format!("/ip4/0.0.0.0/tcp/{}", args.port).parse()?)?;

    // simple provider service of allocated name vars.
    let first_name = args.f_name;
    let last_name = args.l_name;
    // static response request
    let response = Response::new(
        whoami::UserAgent::Server,
        first_name,
        last_name
    );

    // struct async Iter that handle incoming, to which it only accepts the 
    // stream protocol ``/whoami/0.0.1`` 
    let mut incoming_stream = server.behaviour().new_control().accept(WHOAMI_PROTOCOL)?;

    // Spawn deticated thread that handles incoming streams from client.
    // like the stream example in ``rust-libp2p/examples/stream/src`` we are 
    // handling sequential thread.
    // 
    // libp2p doesn't care how you handle incoming streams but you _must_ handle them somehow.
    // To mitigate DoS attacks, libp2p will internally drop incoming streams if your application
    // cannot keep up processing them.
    tokio::spawn(async move {
        // REF: ``rust-libp2p/examples/stream/sr``
        // This loop handles incoming streams _sequentially_ but that doesn't have to be the case.
        // You can also spawn a dedicated task per stream if you want to.
        // Be aware that this breaks backpressure though as spawning new tasks is equivalent to an
        // unbounded buffer. Each task needs memory meaning an aggressive remote peer may
        // force you OOM this way.
        while let Some((peer, stream)) = incoming_stream.next().await {
            // On incoming stream take the stream handed from the incoming_stream
            // send [`Repsonse`] to said stream
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

    // SwarmEvent busy loop of main thread.
    loop {
        tokio::select! {
            event = server.select_next_some() => {
                match event {
                    libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                        let addr = address.with_p2p(*server.local_peer_id()).unwrap();
                        tracing::info!(%addr);
                    }
        
                    event => tracing::trace!(?event),
                }
            }
        }
    }
}
