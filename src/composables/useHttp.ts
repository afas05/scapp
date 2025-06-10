import { fetch } from '@tauri-apps/plugin-http';

export async function useHttp(method: string, url: string, data?: object) {
    const baseUrl: string = 'https://streamsnipe.live/api/';
    const fullUrl: string = baseUrl + url;
    const params: RequestInit = {
        method: method,
        headers: {},
        body: null,
    };

    if (data) {
        params.headers = {
            'Content-Type': 'application/json',
            'Accept': 'application/json'
        };
        params.body = JSON.stringify(data);
    }

    return await fetch(fullUrl, params);
}
