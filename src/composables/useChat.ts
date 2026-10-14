import { ref, computed } from 'vue';
import { useHttp } from './useHttp';
import { useUserStore } from '../stores/userStore';

// ChatMessage::toWire on the server; sent_at is epoch milliseconds.
export interface ChatMessage {
    id: number;
    user_id: number;
    name: string;
    body: string;
    sent_at: number;
}

// {expires_at: ISO string | null} while the user is banned (null) or timed out.
export interface ChatBan {
    expires_at: string | null;
}

export type ChatEvent = { message: ChatMessage } | { deleted: number[] };

export const CHAT_MAX_LENGTH = 300;
export const CHAT_TIMEOUT_MINUTES = 10;
const MAX_KEPT = 200;
const NOTICE_MS = 4000;
// How often a timeout's remaining time is re-checked: drives the countdown and
// lifts the timeout. A single long setTimeout would overflow past ~24.8 days.
const BAN_TICK_MS = 15000;

// Live chat for the broadcast this app is producing. Mirrors the web watch page's
// LiveChat.vue: history once over REST, then SFU pushes (ChatMessage/ChatDeleted)
// on top. Created once by Start.vue and fed pushes from the moment the RPC socket
// opens, so anything that lands before the history (or before the panel mounts)
// is held and replayed rather than lost.
export function useChat() {
    const userStore = useUserStore();

    // Keyed by id: our own message comes back twice (POST response and the SFU
    // echo), and a Map keeps insertion order for rendering.
    const messageMap = ref(new Map<number, ChatMessage>());
    const messages = computed(() => [...messageMap.value.values()]);
    const ready = ref(false);
    const offline = ref(false);
    const canModerate = ref(false);
    const myBan = ref<ChatBan | null>(null);
    // Whole minutes left on a timeout (0 when not timed out), for the composer.
    const banMinutesLeft = ref(0);
    const sending = ref(false);
    const notice = ref('');
    // Bumped when a message is added, so the panel can follow the conversation;
    // `ownAppended` marks our own sends, which always scroll into view.
    const appended = ref(0);
    const ownAppended = ref(0);

    let producerId = '';
    let pending: ChatEvent[] = [];
    // Bumped on every open/close, so history that lands after the broadcast moved
    // on is dropped.
    let generation = 0;
    let banTimer: number | undefined;
    let noticeTimer: number | undefined;

    const canSend = computed(() => ready.value && !offline.value && !myBan.value && !!producerId);

    function flash(text: string) {
        notice.value = text;
        clearTimeout(noticeTimer);
        noticeTimer = window.setTimeout(() => { notice.value = ''; }, NOTICE_MS);
    }

    // A timeout lifts on its own; re-open the composer when it does.
    function checkBan() {
        const expires = myBan.value?.expires_at ? Date.parse(myBan.value.expires_at) : NaN;
        if (Number.isNaN(expires)) {
            banMinutesLeft.value = 0;
            return;
        }
        const left = expires - Date.now();
        if (left <= 0) {
            setBan(null);
            return;
        }
        banMinutesLeft.value = Math.ceil(left / 60000);
    }

    function setBan(ban: ChatBan | null) {
        myBan.value = ban;
        clearInterval(banTimer);
        banTimer = undefined;
        banMinutesLeft.value = 0;
        if (ban?.expires_at) {
            checkBan();
            if (myBan.value) banTimer = window.setInterval(checkBan, BAN_TICK_MS);
        }
    }

    function addMessage(msg: ChatMessage) {
        const map = messageMap.value;
        const isNew = !map.has(msg.id);
        map.set(msg.id, msg);
        while (map.size > MAX_KEPT) map.delete(map.keys().next().value!);
        if (isNew) appended.value++;
    }

    function removeMessages(ids: number[]) {
        for (const id of ids) messageMap.value.delete(id);
    }

    function apply(event: ChatEvent) {
        if ('message' in event) addMessage(event.message);
        else removeMessages(event.deleted);
    }

    // SFU push. A message already in the history just re-sets the same entry.
    function onPush(event: ChatEvent) {
        if (ready.value) apply(event);
        else pending.push(event);
    }

    async function request(method: string, url: string, data?: object) {
        const res = await useHttp(method, url, data, userStore.sessionHash);
        let body: any = null;
        try { body = await res.json(); } catch { /* 204 or non-JSON */ }
        return { status: res.status, ok: res.ok, body };
    }

    // A new broadcast (new producer id) is a new session, loaded fresh. The SFU
    // writes the session row before acknowledging Produce, so no retry is needed.
    async function open(id: string) {
        const myGeneration = ++generation;
        producerId = id;
        messageMap.value = new Map();
        ready.value = false;
        offline.value = false;
        canModerate.value = false;
        setBan(null);

        let data: any = null;
        try {
            const res = await request('GET', `streams/${encodeURIComponent(id)}/chat`);
            if (res.ok) data = res.body;
            else console.warn('[Chat] history failed:', res.status);
        } catch (err) {
            console.warn('[Chat] history failed:', err);
        }
        if (myGeneration !== generation) return;

        // A failed load still leaves a usable chat: everything said from here on.
        const history: ChatMessage[] = (data?.messages ?? []).slice(-MAX_KEPT);
        messageMap.value = new Map(history.map((m) => [m.id, m]));
        canModerate.value = Boolean(data?.can_moderate);
        if (data && data.live === false) offline.value = true;
        setBan(data?.my_ban ?? null);
        for (const event of pending) apply(event);
        pending = [];
        ready.value = true;
        appended.value++;
    }

    function close() {
        generation++;
        producerId = '';
        pending = [];
        ready.value = false;
        setBan(null);
        clearTimeout(noticeTimer);
        notice.value = '';
    }

    // Returns true when the message was accepted, so the caller can clear the draft.
    async function send(text: string): Promise<boolean> {
        const body = text.trim();
        if (!body || !canSend.value || sending.value) return false;
        sending.value = true;
        try {
            const res = await request('POST', `streams/${encodeURIComponent(producerId)}/chat`, { body });
            if (res.status === 201 && res.body) {
                ownAppended.value++;
                addMessage(res.body as ChatMessage);
                return true;
            }
            const data = res.body ?? {};
            if (res.status === 429) flash(`Slow down — try again in ${data.retry_after ?? 'a few'}s`);
            else if (res.status === 403 && data.ban !== undefined) setBan(data.ban);
            else if (res.status === 409) { offline.value = true; flash('The stream is offline.'); }
            else if (res.status === 422) flash(data.message ?? 'That message can’t be sent.');
            else flash('Message not sent.');
        } catch (err) {
            console.warn('[Chat] send failed:', err);
            flash('Message not sent.');
        } finally {
            sending.value = false;
        }
        return false;
    }

    function canBan(msg: ChatMessage) {
        return msg.user_id !== userStore.userId;
    }

    async function remove(msg: ChatMessage) {
        try {
            const res = await request('DELETE', `chat/messages/${msg.id}`);
            if (!res.ok) throw new Error(`HTTP ${res.status}`);
            removeMessages([msg.id]);
        } catch (err) {
            console.warn('[Chat] delete failed:', err);
            flash('Couldn’t delete that message.');
        }
    }

    // minutes: a timeout length, or null to ban until lifted.
    async function ban(msg: ChatMessage, minutes: number | null) {
        const what = minutes ? 'time out' : 'ban';
        if (!userStore.userId) {
            flash(`Couldn’t ${what} ${msg.name}: log in again to enable moderation.`);
            return;
        }
        try {
            const res = await request('POST', `channels/${userStore.userId}/bans`, { user_id: msg.user_id, minutes });
            if (!res.ok) throw new Error(`HTTP ${res.status}`);
            // The SFU push clears their messages for everyone; do it here too, at once.
            removeMessages(messages.value.filter((m) => m.user_id === msg.user_id).map((m) => m.id));
            flash(minutes ? `${msg.name} timed out for ${minutes} min.` : `${msg.name} is banned from chat.`);
        } catch (err) {
            console.warn('[Chat] ban failed:', err);
            flash(`Couldn’t ${what} ${msg.name}.`);
        }
    }

    return {
        messages, ready, offline, canModerate, myBan, banMinutesLeft, sending, notice, canSend,
        appended, ownAppended,
        onPush, open, close, send, remove, ban, canBan,
    };
}

export type Chat = ReturnType<typeof useChat>;
