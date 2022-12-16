extern crate canadensis_core;
extern crate canadensis_linux;
extern crate canadensis_udp;
extern crate simplelog;

use canadensis_core::session::SessionDynamicMap;
use canadensis_core::time::{Clock, MicrosecondDuration64, Microseconds64};
use canadensis_core::transfer::{Header, MessageHeader, Transfer};
use canadensis_core::transport::{Receiver, TransferId, Transmitter};
use canadensis_core::{Priority, SubjectId};
use canadensis_linux::SystemClock;
use canadensis_udp::{
    UdpNodeId, UdpReceiver, UdpSessionData, UdpTransferId, UdpTransmitter, DEFAULT_PORT,
};
use log::LevelFilter;
use simplelog::{ColorChoice, TermLogger, TerminalMode};
use std::convert::{TryFrom, TryInto};
use std::net::Ipv4Addr;
use std::thread::sleep;
use std::time::{Duration, Instant};

#[test]
fn transmit_receive_message_two_frames() {
    TermLogger::init(
        LevelFilter::Trace,
        Default::default(),
        TerminalMode::Stderr,
        ColorChoice::Auto,
    )
    .unwrap();

    let transmit_node_id = UdpNodeId::try_from(120).unwrap();
    let receive_node_id = UdpNodeId::try_from(1009).unwrap();
    let mut clock = SystemClock::new();
    const MTU: usize = 1472;

    let mut receiver = UdpReceiver::<
        Microseconds64,
        SessionDynamicMap<Microseconds64, UdpNodeId, UdpTransferId, UdpSessionData>,
        MTU,
    >::new(Some(receive_node_id), Ipv4Addr::LOCALHOST, DEFAULT_PORT)
    .expect("Failed to create receiver");
    receiver
        .subscribe_message(
            73.try_into().unwrap(),
            4096,
            MicrosecondDuration64::new(2_000_000),
            &mut (),
        )
        .unwrap();

    let mut transmitter = UdpTransmitter::<MTU>::new(Ipv4Addr::LOCALHOST, DEFAULT_PORT).unwrap();

    // Make a payload compatible with the uavcan.metatransport.ethernet.Frame.0.1 format format.
    let payload = {
        let mut payload = Vec::with_capacity(6 + 6 + 2 + 2 + MAJOR_GENERAL_SONG.len());
        // Destination
        payload.extend_from_slice(&[0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6]);
        // Source
        payload.extend_from_slice(&[0x31, 0x32, 0x33, 0x34, 0x35, 0x36]);
        // Type IPv4
        payload.extend_from_slice(&[0x00, 0x08]);
        let length: u16 = MAJOR_GENERAL_SONG.len().try_into().unwrap();
        payload.extend_from_slice(&[length as u8, (length >> 8) as u8]);
        payload.extend_from_slice(MAJOR_GENERAL_SONG);
        payload
    };

    let mut transfer_id = UdpTransferId::default();
    for _ in 0..10 {
        let transfer = Transfer {
            header: Header::Message(MessageHeader {
                timestamp: MicrosecondDuration64::new(1_000_000) + clock.now(),
                transfer_id: transfer_id.clone(),
                priority: Priority::Nominal,
                subject: SubjectId::try_from(73u16).unwrap(),
                source: Some(transmit_node_id),
            }),
            payload: &payload,
        };

        transmitter.push(transfer, &mut clock, &mut ()).unwrap();
        transmitter.flush(&mut clock, &mut ()).unwrap();

        let timeout = Instant::now() + Duration::from_secs(1);
        loop {
            match receiver.receive(clock.now(), &mut ()) {
                Ok(Some(transfer)) => {
                    assert_eq!(transfer.payload, payload);
                    break;
                }
                Ok(None) => {
                    sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("Receive error {:?}", e),
            }

            if Instant::now() > timeout {
                panic!("Timed out waiting for receive");
            }
        }

        transfer_id = transfer_id.increment();
    }
}

const MAJOR_GENERAL_SONG: &[u8] = br#"I am the very model of a modern Major-Gineral,
I've information vegetable, animal, and mineral,
I know the kings of England, and I quote the fights historical
From Marathon to Waterloo, in order categorical;
I'm very well acquainted, too, with matters mathematical,
I understand equations, both the simple and quadratical,
About binomial theorem I'm teeming with a lot o' news,
With many cheerful facts about the square of the hypotenuse.
I'm very good at integral and differential calculus;
I know the scientific names of beings animalculous:
In short, in matters vegetable, animal, and mineral,
I am the very model of a modern Major-Gineral.
I know our mythic history, King Arthur's and Sir Caradoc's;
I answer hard acrostics, I've a pretty taste for paradox,
I quote in elegiacs all the crimes of Heliogabalus,
In conics I can floor peculiarities parabolous;
I can tell undoubted Raphaels from Gerard Dows and Zoffanies,
I know the croaking chorus from The Frogs of Aristophanes!
Then I can hum a fugue of which I've heard the music's din afore,
And whistle all the airs from that infernal nonsense Pinafore.
Then I can write a washing bill in Babylonic cuneiform,
And tell you ev'ry detail of Caractacus's uniform:
In short, in matters vegetable, animal, and mineral,
I am the very model of a modern Major-Gineral.
In fact, when I know what is meant by "mamelon" and "ravelin",
When I can tell at sight a Mauser rifle from a javelin,
When such affairs as sorties and surprises I'm more wary at,
And when I know precisely what is meant by "commissariat",
When I have learnt what progress has been made in modern gunnery,
When I know more of tactics than a novice in a nunnery -
In short, when I've a smattering of elemental strategy -
You'll say a better Major-General has never sat a gee.
For my military knowledge, though I'm plucky and adventury,
Has only been brought down to the beginning of the century;
But still, in matters vegetable, animal, and mineral,
I am the very model of a modern Major-Gineral."#;
