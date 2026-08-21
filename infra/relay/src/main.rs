use std::{env, fs, path::PathBuf};

use futures::StreamExt;
use libp2p::{
    identify,
    identity::Keypair,
    noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, SwarmBuilder,
};

#[derive(NetworkBehaviour)]
struct Behaviour {
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    relay: relay::Behaviour,
}

fn load_or_create_identity() -> Result<Keypair, Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        env::var("TEAMSCORD_RELAY_IDENTITY_PATH")
            .unwrap_or_else(|_| "/app/data/identity.bin".into()),
    );
    if let Ok(bytes) = fs::read(&path) {
        return Ok(Keypair::from_protobuf_encoding(&bytes)?);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let keypair = Keypair::generate_ed25519();
    fs::write(path, keypair.to_protobuf_encoding()?)?;
    Ok(keypair)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let keypair = load_or_create_identity()?;
    let peer_id = keypair.public().to_peer_id();
    let tcp_port = env::var("TEAMSCORD_RELAY_TCP_PORT").unwrap_or_else(|_| "4001".into());
    let quic_port = env::var("TEAMSCORD_RELAY_QUIC_PORT").unwrap_or_else(|_| "4002".into());

    let mut swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_behaviour(|key| Behaviour {
            identify: identify::Behaviour::new(identify::Config::new(
                "/teamscord/relay/1".into(),
                key.public(),
            )),
            ping: ping::Behaviour::new(ping::Config::new()),
            relay: relay::Behaviour::new(peer_id, relay::Config::default()),
        })
        .expect("relay behaviour is infallible")
        .build();

    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{tcp_port}").parse()?)?;
    swarm.listen_on(format!("/ip4/0.0.0.0/udp/{quic_port}/quic-v1").parse()?)?;
    println!("Teamscord relay online: {peer_id}");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = swarm.next() => match event {
                Some(SwarmEvent::NewListenAddr { address, .. }) => println!("listening on {address}"),
                Some(SwarmEvent::Behaviour(event)) => println!("relay event: {event:?}"),
                Some(_) => {}
                None => break,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use libp2p::{
        multiaddr::{Multiaddr, Protocol},
        swarm::Swarm,
    };
    use std::time::Duration;

    #[derive(NetworkBehaviour)]
    struct ClientBehaviour {
        ping: ping::Behaviour,
        relay: relay::client::Behaviour,
    }

    fn build_server() -> Swarm<Behaviour> {
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .expect("relay TCP transport")
            .with_behaviour(|key| Behaviour {
                identify: identify::Behaviour::new(identify::Config::new(
                    "/teamscord/relay/1".into(),
                    key.public(),
                )),
                ping: ping::Behaviour::new(ping::Config::new()),
                relay: relay::Behaviour::new(peer_id, relay::Config::default()),
            })
            .expect("relay behaviour")
            .build()
    }

    fn build_client() -> Swarm<ClientBehaviour> {
        SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .expect("client TCP transport")
            .with_relay_client(noise::Config::new, yamux::Config::default)
            .expect("relay client transport")
            .with_behaviour(|_, relay| ClientBehaviour {
                ping: ping::Behaviour::new(ping::Config::new()),
                relay,
            })
            .expect("client behaviour")
            .build()
    }

    async fn wait_for_reservation(
        client: &mut Swarm<ClientBehaviour>,
        expected_address: &Multiaddr,
        relay_peer_id: libp2p::PeerId,
    ) {
        let mut address_reported = false;
        let mut reservation_accepted = false;
        loop {
            match client.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. } if &address == expected_address => {
                    address_reported = true;
                }
                SwarmEvent::Behaviour(ClientBehaviourEvent::Relay(
                    relay::client::Event::ReservationReqAccepted {
                        relay_peer_id: peer_id,
                        ..
                    },
                )) if peer_id == relay_peer_id => {
                    reservation_accepted = true;
                }
                _ => {}
            }
            if address_reported && reservation_accepted {
                return;
            }
        }
    }

    async fn wait_for_peer(client: &mut Swarm<ClientBehaviour>, peer_id: libp2p::PeerId) {
        loop {
            if let SwarmEvent::ConnectionEstablished {
                peer_id: connected_peer,
                ..
            } = client.select_next_some().await
            {
                if connected_peer == peer_id {
                    return;
                }
            }
        }
    }

    #[tokio::test]
    async fn two_clients_exchange_connection_through_local_relay() {
        let mut server = build_server();
        server
            .listen_on("/ip4/127.0.0.1/tcp/0".parse().expect("server address"))
            .expect("server listen");
        let server_address = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let SwarmEvent::NewListenAddr { address, .. } = server.select_next_some().await {
                    break address;
                }
            }
        })
        .await
        .expect("relay listen timeout");
        let server_peer_id = *server.local_peer_id();
        server.add_external_address(server_address.clone());
        let server_task = tokio::spawn(async move { while server.next().await.is_some() {} });

        let mut first = build_client();
        let mut second = build_client();
        let first_peer_id = *first.local_peer_id();
        let second_peer_id = *second.local_peer_id();
        let relay_address = server_address.with(Protocol::P2p(server_peer_id));
        let first_relay_address = relay_address.clone().with(Protocol::P2pCircuit);
        let second_relay_address = relay_address.clone().with(Protocol::P2pCircuit);
        first
            .listen_on(first_relay_address.clone())
            .expect("first reservation");
        second
            .listen_on(second_relay_address.clone())
            .expect("second reservation");

        let expected_first_address = first_relay_address.with(Protocol::P2p(first_peer_id));
        let expected_second_address = second_relay_address.with(Protocol::P2p(second_peer_id));
        tokio::time::timeout(
            Duration::from_secs(10),
            futures::future::join(
                wait_for_reservation(&mut first, &expected_first_address, server_peer_id),
                wait_for_reservation(&mut second, &expected_second_address, server_peer_id),
            ),
        )
        .await
        .expect("relay reservation timeout");

        first
            .dial(expected_second_address)
            .expect("dial second client through relay");
        tokio::time::timeout(
            Duration::from_secs(10),
            futures::future::join(
                wait_for_peer(&mut first, second_peer_id),
                wait_for_peer(&mut second, first_peer_id),
            ),
        )
        .await
        .expect("relay circuit timeout");

        server_task.abort();
    }
}
