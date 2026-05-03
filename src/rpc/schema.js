import { defineSchema } from 'kiss-rpc-fb';
import { Method } from './generated/mediasoup-signaling/method';
import {
    CreateProducerTransportRequest,
    CreateProducerTransportResponse,
    ConnectProducerTransportRequest,
    ConnectProducerTransportResponse,
    ProduceRequest,
    ProduceResponse,
    CreateConsumerTransportRequest,
    CreateConsumerTransportResponse,
    ConnectConsumerTransportRequest,
    ConnectConsumerTransportResponse,
    ConsumeRequest,
    ConsumeResponse,
    ResumeRequest,
    ResumeResponse,
    ProducerClosedRequest,
    ViewerCountRequest,
    StreamEndedRequest,
    CreateProducerPlainTransportRequest,
    CreateProducerPlainTransportResponse,
} from './generated/mediasoup-signaling_generated';

export { Method };

export const schema = defineSchema({
    [Method.CreateProducerTransport]: { Req: CreateProducerTransportRequest, Res: CreateProducerTransportResponse },
    [Method.ConnectProducerTransport]: { Req: ConnectProducerTransportRequest, Res: ConnectProducerTransportResponse },
    [Method.Produce]: { Req: ProduceRequest, Res: ProduceResponse },
    [Method.CreateConsumerTransport]: { Req: CreateConsumerTransportRequest, Res: CreateConsumerTransportResponse },
    [Method.ConnectConsumerTransport]: { Req: ConnectConsumerTransportRequest, Res: ConnectConsumerTransportResponse },
    [Method.Consume]: { Req: ConsumeRequest, Res: ConsumeResponse },
    [Method.Resume]: { Req: ResumeRequest, Res: ResumeResponse },
    [Method.ProducerClosed]: { Req: ProducerClosedRequest },
    [Method.ViewerCount]: { Req: ViewerCountRequest },
    [Method.StreamEnded]: { Req: StreamEndedRequest },
    [Method.CreateProducerPlainTransport]: { Req: CreateProducerPlainTransportRequest, Res: CreateProducerPlainTransportResponse },
});
