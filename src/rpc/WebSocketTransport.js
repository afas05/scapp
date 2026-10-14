import { FbRpc } from 'kiss-rpc-fb';
import { schema } from './schema';

// onClose fires once the socket closes after having opened, whoever closed it.
export function createWebSocketTransport(url, token, onClose = null) {
    const rpc = new FbRpc(schema);

    return new Promise((resolve, reject) => {
        const ws = new WebSocket(url + token);
        ws.binaryType = 'arraybuffer';
        let opened = false;

        ws.onopen = () => {
            opened = true;
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
            if (opened && onClose) onClose();
        };
    });
}
