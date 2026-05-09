import { fetch } from '@tauri-apps/plugin-http';

export async function useHttp(method: string, url: string, data?: object, token?: string) {
    const baseUrl: string = 'https://streamsnipe.live/api/';
    const fullUrl: string = baseUrl + url;
    const headers: Record<string, string> = {
        'Accept': 'application/json',
    };
    const params: RequestInit = {
        method: method,
        headers,
        body: null,
    };

    if (data) {
        headers['Content-Type'] = 'application/json';
        params.body = JSON.stringify(data);
    }

    if (token) {
        headers['Authorization'] = `Bearer ${token}`;
    }

    return await fetch(fullUrl, params);
}
