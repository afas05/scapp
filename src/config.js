import { Store } from '@tauri-apps/plugin-store';

const STORE_PATH = 'config.json';
const DEFAULTS = {
    sfuWsUrl:   'wss://streamsnipe.live:3000',
    sfuHttpUrl: 'https://streamsnipe.live:3000',
};

export async function loadConfig() {
    const store = await Store.load(STORE_PATH);
    return {
        sfuWsUrl:   (await store.get('sfuWsUrl'))   ?? DEFAULTS.sfuWsUrl,
        sfuHttpUrl: (await store.get('sfuHttpUrl')) ?? DEFAULTS.sfuHttpUrl,
    };
}

export async function saveConfig(values) {
    const store = await Store.load(STORE_PATH);
    for (const [k, v] of Object.entries(values)) await store.set(k, v);
    await store.save();
}
