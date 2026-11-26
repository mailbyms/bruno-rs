//! tonic 0.14 把 prost 编解码拆到了 `tonic-prost` 独立 crate，
//! 这里只需要「任意 prost 消息」和「DynamicMessage」两种编解码器，直接手写。

use prost::Message as ProstMessage;
use prost_reflect::{DynamicMessage, MessageDescriptor};
use std::marker::PhantomData;
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::Status;

// ---------------------------------------------------------------------------
// 任意 prost::Message
// ---------------------------------------------------------------------------

pub struct ProstCodec<M1, M2>(PhantomData<(M1, M2)>);

impl<M1, M2> Default for ProstCodec<M1, M2> {
    fn default() -> Self {
        ProstCodec(PhantomData)
    }
}

pub struct ProstEncoder<M>(PhantomData<M>);
pub struct ProstDecoder<M>(PhantomData<M>);

impl<M1, M2> Codec for ProstCodec<M1, M2>
where
    M1: ProstMessage + Send + 'static,
    M2: ProstMessage + Default + Send + 'static,
{
    type Encode = M1;
    type Decode = M2;
    type Encoder = ProstEncoder<M1>;
    type Decoder = ProstDecoder<M2>;

    fn encoder(&mut self) -> Self::Encoder {
        ProstEncoder(PhantomData)
    }

    fn decoder(&mut self) -> Self::Decoder {
        ProstDecoder(PhantomData)
    }
}

impl<M: ProstMessage> Encoder for ProstEncoder<M> {
    type Item = M;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst).map_err(|e| Status::internal(format!("编码失败：{e}")))
    }
}

impl<M: ProstMessage + Default> Decoder for ProstDecoder<M> {
    type Item = M;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        M::decode(src)
            .map(Some)
            .map_err(|e| Status::internal(format!("解码失败：{e}")))
    }
}

// ---------------------------------------------------------------------------
// DynamicMessage（输出描述符在创建 codec 时确定）
// ---------------------------------------------------------------------------

pub struct DynCodec {
    pub output: MessageDescriptor,
}

pub struct DynEncoder;
pub struct DynDecoder {
    output: Option<MessageDescriptor>,
}

impl Codec for DynCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynEncoder;
    type Decoder = DynDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynDecoder {
            output: Some(self.output.clone()),
        }
    }
}

impl Encoder for DynEncoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Self::Error> {
        item.encode(dst)
            .map_err(|e| Status::internal(format!("编码 gRPC 消息失败：{e}")))
    }
}

impl Decoder for DynDecoder {
    type Item = DynamicMessage;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Self::Error> {
        let Some(desc) = self.output.take() else {
            return Ok(None);
        };
        DynamicMessage::decode(desc, src)
            .map(Some)
            .map_err(|e| Status::internal(format!("解码 gRPC 响应失败：{e}")))
    }
}
