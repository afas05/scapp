<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted } from 'vue';
import Icon from '../shared/Icon.vue';
import { useUserStore } from '../../stores/userStore';
import { CHAT_MAX_LENGTH, CHAT_TIMEOUT_MINUTES, type Chat, type ChatMessage } from '../../composables/useChat';

const props = defineProps<{
  chat: Chat;
}>();

// How close to the bottom still counts as "reading the latest".
const STICK_PX = 40;

const userStore = useUserStore();
const collapsed = ref(false);
const draft = ref('');
const unread = ref(false);
const scrollEl = ref<HTMLElement | null>(null);
const inputEl = ref<HTMLInputElement | null>(null);
let atBottom = true;

const messages = computed(() => props.chat.messages.value);

const placeholder = computed(() => {
  const chat = props.chat;
  if (!chat.ready.value) return 'Loading chat…';
  if (chat.offline.value) return 'Stream is offline';
  const ban = chat.myBan.value;
  if (ban) {
    if (!ban.expires_at) return 'You are banned from this chat';
    return `Timed out for ${Math.max(1, chat.banMinutesLeft.value)} min`;
  }
  return 'Say something to your viewers';
});

function nameColor(userId: number) {
  const hue = (userId * 137) % 360;
  return `oklch(0.78 0.13 ${hue})`;
}

function formatTime(ms: number) {
  return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function isMine(msg: ChatMessage) {
  return msg.user_id === userStore.userId;
}

// --- Scroll ------------------------------------------------------------------------

function onScroll() {
  const el = scrollEl.value;
  if (!el) return;
  atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX;
  if (atBottom) unread.value = false;
}

function scrollToBottom(behavior: ScrollBehavior = 'auto') {
  const el = scrollEl.value;
  if (!el) return;
  el.scrollTo({ top: el.scrollHeight, behavior });
  atBottom = true;
  unread.value = false;
}

// Follow the conversation only when already at the bottom; someone scrolled up
// reading history gets a pill instead of being yanked down.
watch(() => props.chat.appended.value, async () => {
  if (collapsed.value) {
    unread.value = true;
    return;
  }
  const follow = atBottom;
  await nextTick();
  if (follow) scrollToBottom();
  else unread.value = true;
});

// Your own message always scrolls into view.
watch(() => props.chat.ownAppended.value, () => { atBottom = true; });

// A collapsed panel has no list to scroll, so catch up when it is opened again.
watch(collapsed, async (c) => {
  if (c) return;
  await nextTick();
  scrollToBottom();
});

onMounted(() => scrollToBottom());

// --- Sending -----------------------------------------------------------------------

async function send() {
  if (await props.chat.send(draft.value)) draft.value = '';
  if (props.chat.canSend.value) nextTick(() => inputEl.value?.focus());
}
</script>

<template>
  <aside class="chat" :class="{ collapsed }">
    <button
      v-if="collapsed"
      type="button"
      class="rail"
      title="Show chat"
      aria-label="Show chat"
      @click="collapsed = false"
    >
      <Icon name="chat" :size="13" />
      <span v-if="unread" class="rail-dot" />
    </button>

    <template v-else>
      <header class="chat-head">
        <Icon name="chat" :size="11" style="color: var(--text-dim);" />
        <span class="section-label">Chat</span>
        <button
          type="button"
          class="collapse-btn"
          title="Hide chat"
          aria-label="Hide chat"
          @click="collapsed = true"
        >
          <Icon name="chev" :size="11" />
        </button>
      </header>

      <div ref="scrollEl" class="ws-scroll chat-stream" @scroll="onScroll">
        <p v-if="chat.ready.value && messages.length === 0" class="chat-empty">
          No messages yet. Chat from your viewers shows up here.
        </p>
        <div v-for="msg in messages" :key="msg.id" class="chat-msg">
          <div class="msg-body" :title="formatTime(msg.sent_at)">
            <span class="name" :style="{ color: nameColor(msg.user_id) }">{{ msg.name }}</span>
            <span v-if="isMine(msg)" class="badge">you</span>
            <span class="text">{{ msg.body }}</span>
          </div>
          <div v-if="chat.canModerate.value" class="mod">
            <button type="button" class="mod-btn" title="Delete message" :aria-label="`Delete message from ${msg.name}`" @click="chat.remove(msg)">
              <Icon name="trash" :size="11" />
            </button>
            <template v-if="chat.canBan(msg)">
              <button type="button" class="mod-btn" :title="`Time out ${CHAT_TIMEOUT_MINUTES} min`" :aria-label="`Time out ${msg.name}`" @click="chat.ban(msg, CHAT_TIMEOUT_MINUTES)">
                <Icon name="clock" :size="11" />
              </button>
              <button type="button" class="mod-btn danger" title="Ban from chat" :aria-label="`Ban ${msg.name}`" @click="chat.ban(msg, null)">
                <Icon name="ban" :size="11" />
              </button>
            </template>
          </div>
        </div>
      </div>

      <button v-if="unread" type="button" class="new-pill" @click="scrollToBottom('smooth')">
        New messages
      </button>

      <p v-if="chat.notice.value" class="chat-notice" role="status">{{ chat.notice.value }}</p>

      <form class="chat-input" @submit.prevent="send">
        <input
          ref="inputEl"
          v-model="draft"
          type="text"
          :maxlength="CHAT_MAX_LENGTH"
          :placeholder="placeholder"
          :disabled="!chat.canSend.value"
          aria-label="Chat message"
          autocomplete="off"
        />
        <span v-if="draft.length > CHAT_MAX_LENGTH - 50" class="counter ws-mono">{{ CHAT_MAX_LENGTH - draft.length }}</span>
        <button
          type="submit"
          class="send"
          :disabled="!chat.canSend.value || chat.sending.value || !draft.trim()"
        >
          Send
        </button>
      </form>
    </template>
  </aside>
</template>

<style scoped>
.chat {
  position: relative;
  flex: 0 0 280px;
  width: 280px;
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: var(--bg-deep);
  border-left: 1px solid var(--border-soft);
}
.chat.collapsed {
  flex-basis: 36px;
  width: 36px;
}

.rail {
  position: relative;
  display: flex;
  justify-content: center;
  padding: 12px 0;
  background: transparent;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
}
.rail:hover { color: var(--text); }
.rail-dot {
  position: absolute;
  top: 9px;
  right: 9px;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
}

.chat-head {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 10px 0 14px;
  height: 45px;
  border-bottom: 1px solid var(--border-soft);
}
.section-label {
  font-size: 9px;
  color: var(--text-mute);
  letter-spacing: 0.6px;
  text-transform: uppercase;
  font-weight: 500;
}
.collapse-btn {
  margin-left: auto;
  display: inline-flex;
  padding: 4px;
  border-radius: 5px;
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  cursor: pointer;
}

.chat-stream {
  flex: 1 1 0;
  min-height: 0;
  overflow-y: auto;
  padding: 8px 0;
}
.chat-empty {
  margin: 0;
  padding: 16px 14px;
  font-size: 11px;
  color: var(--text-mute);
  text-align: center;
}
.chat-msg {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: 4px;
  padding: 3px 14px;
}
.chat-msg:hover { background: var(--surface-1); }
.msg-body {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  line-height: 1.4;
  color: var(--text);
  overflow-wrap: anywhere;
}
.name {
  font-weight: 600;
  margin-right: 4px;
}
.badge {
  display: inline-block;
  margin-right: 4px;
  padding: 0 4px;
  border-radius: 3px;
  font-size: 9px;
  line-height: 14px;
  vertical-align: 1px;
  text-transform: uppercase;
  letter-spacing: 0.4px;
  color: var(--accent);
  border: 1px solid var(--accent-soft);
}

.mod {
  display: none;
  flex-shrink: 0;
  gap: 2px;
}
.chat-msg:hover .mod { display: flex; }
.mod-btn {
  display: inline-flex;
  padding: 3px;
  border-radius: 4px;
  background: var(--surface-2);
  border: 1px solid var(--border);
  color: var(--text-dim);
  cursor: pointer;
}
.mod-btn:hover { color: var(--text); }
.mod-btn.danger:hover { color: var(--rec); border-color: var(--rec); }

.new-pill {
  position: absolute;
  left: 50%;
  bottom: 56px;
  transform: translateX(-50%);
  padding: 4px 10px;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  border: none;
  font-size: 10px;
  font-family: inherit;
  cursor: pointer;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
}

.chat-notice {
  flex-shrink: 0;
  margin: 0;
  padding: 6px 14px;
  font-size: 10px;
  color: var(--warn);
  border-top: 1px solid var(--border-soft);
}

.chat-input {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px;
  border-top: 1px solid var(--border-soft);
}
.chat-input input {
  flex: 1;
  min-width: 0;
  padding: 7px 9px;
  border-radius: 6px;
  background: var(--surface-1);
  border: 1px solid var(--border-soft);
  color: var(--text);
  font-size: 12px;
  font-family: inherit;
  outline: none;
}
.chat-input input:focus { border-color: var(--accent-soft); }
.chat-input input:disabled { opacity: 0.6; }
.counter {
  font-size: 10px;
  color: var(--text-mute);
}
.send {
  padding: 7px 11px;
  border-radius: 6px;
  background: var(--accent);
  color: #fff;
  border: none;
  font-size: 12px;
  font-weight: 500;
  font-family: inherit;
  cursor: pointer;
}
.send:disabled {
  opacity: 0.4;
  cursor: default;
}
</style>
