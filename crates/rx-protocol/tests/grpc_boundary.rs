//! Real loopback gRPC test of pre-deserialization validation.
//! Authentication, deployment TLS and hardware are deliberately outside this fixture.
use bytes::{Buf, BufMut, Bytes};
use prost::Message;
use rx_protocol::base;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{
    Request, Response, Status,
    codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder},
};

struct Probe(Arc<AtomicUsize>);
#[tonic::async_trait]
impl base::session_service_server::SessionService for Probe {
    async fn open(&self, _: Request<base::PeerHello>) -> Result<Response<base::Session>, Status> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Response::new(base::Session {
            peer_id: "test/probe".into(),
            ..Default::default()
        }))
    }
}

#[derive(Default)]
struct RawCodec;
struct RawEncoder;
struct RawDecoder;
impl Codec for RawCodec {
    type Encode = Bytes;
    type Decode = Bytes;
    type Encoder = RawEncoder;
    type Decoder = RawDecoder;
    fn encoder(&mut self) -> RawEncoder {
        RawEncoder
    }
    fn decoder(&mut self) -> RawDecoder {
        RawDecoder
    }
}
impl Encoder for RawEncoder {
    type Item = Bytes;
    type Error = Status;
    fn encode(&mut self, item: Bytes, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        dst.put_slice(&item);
        Ok(())
    }
}
impl Decoder for RawDecoder {
    type Item = Bytes;
    type Error = Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Bytes>, Status> {
        Ok(Some(src.copy_to_bytes(src.remaining())))
    }
}

#[tokio::test]
async fn malformed_wire_never_reaches_generated_service_handler() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (finish, done) = tokio::sync::oneshot::channel();
    let service = base::session_service_server::SessionServiceServer::new(Probe(calls.clone()))
        .max_decoding_message_size(1_048_576);
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async {
                let _ = done.await;
            }),
    );
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .timeout(Duration::from_secs(3))
        .connect()
        .await
        .unwrap();
    let mut client = tonic::client::Grpc::new(channel);
    let path =
        tonic::codegen::http::uri::PathAndQuery::from_static("/rx.contract.v1.SessionService/Open");
    client.ready().await.unwrap();
    let valid = base::PeerHello {
        role: base::Role::Platform as i32,
        ..Default::default()
    }
    .encode_to_vec();
    let _: Response<Bytes> = client
        .unary(
            Request::new(Bytes::from(valid.clone())),
            path.clone(),
            RawCodec,
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut duplicate = valid.clone();
    duplicate.extend_from_slice(&valid);
    client.ready().await.unwrap();
    let error = client
        .unary::<Bytes, Bytes, _>(Request::new(Bytes::from(duplicate)), path.clone(), RawCodec)
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    let mut unknown = valid;
    unknown.extend_from_slice(&[0xa0, 6, 1]);
    client.ready().await.unwrap();
    let error = client
        .unary::<Bytes, Bytes, _>(Request::new(Bytes::from(unknown)), path, RawCodec)
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(client);
    finish.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
