import { fetch } from '@tauri-apps/plugin-http';
import { appVersion, requireUpdate } from '../lib/updater';

export async function useHttp(method: string, url: string, data?: object, token?: string) {
    const baseUrl: string = 'https://streamsnipe.live/api/';
    const fullUrl: string = baseUrl + url;
    const headers: Record<string, string> = {
        'Accept': 'application/json',
        // The server turns away versions below its minimum with a 426.
        'X-App-Version': await appVersion(),
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

    const response = await fetch(fullUrl, params);
    if (response.status === 426) {
        let message: string | undefined;
        try {
            message = (await response.clone().json())?.message;
        } catch { /* no JSON body */ }
        requireUpdate(message);
        // Imported late: the router pulls in views that import this module.
        const { default: router } = await import('../router');
        await router.replace('/update-required');
    }
    return response;
}
