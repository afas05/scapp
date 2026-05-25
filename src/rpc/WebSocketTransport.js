import { FbRpc } from 'kiss-rpc-fb';
import { schema } from './schema';

export function createWebSocketTransport(url, token) {
    const rpc = new FbRpc(schema);

    return new Promise((resolve, reject) => {
        const ws = new WebSocket(url + token);
        ws.binaryType = 'arraybuffer';

        ws.onopen = () => {
            rpc.registerToTransportCallback((data) => {
                ws.send(data);
            });

            ws.onmessage = (event) => {
                rpc.fromTransport(new Uint8Array(event.data));
            };

            resolve({ rpc, close: () => ws.close() });
        };

        ws.onerror = () => reject(new Error('WebSocket connection failed (check auth token or server availability)'));

        ws.onclose = () => {
            rpc.clean('WebSocket closed');
        };
    });
}
