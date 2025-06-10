export function useWS(sessionHash:string) {
    const connectUrl: string = 'wss://streamsnipe.live:3000?auth=' + sessionHash;
    return new WebSocket(connectUrl);
}
