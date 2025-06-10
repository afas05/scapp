<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open } from '@tauri-apps/plugin-shell';
import { useUserStore } from "./stores/userStore";
import { useWS } from "./composables/useWS.ts";

const connectionState = ref(false);
const connectProcess = ref(false);
const state = ref(0);
const producerId = ref('');
const userStore = useUserStore();
const ws = useWS(userStore.sessionHash);

ws.onmessage = async (event) => {
  const msg = JSON.parse(event.data);

  switch (msg.type) {
    case 'producerTransportCreated':
      state.value = 1;
      ws.send(JSON.stringify({ type: 'desktopProduce', kind: 'video', payloadType: 100 }))
      break;

    case 'plainProducerTransportConnected':
      state.value = 2;
      ws.send(JSON.stringify({ type: 'desktopProduce', kind: 'video', payloadType: 100 }))
      break;

    case 'producedCreated':
      connectionState.value = true;
      state.value = 3;
      producerId.value = msg.id;
      state.value = 4;
      await invoke("start_stream");
      break;
  }
};

async function startStream() {
  connectProcess.value = true;
  ws.send(JSON.stringify({ type: 'createPlainTransport' }))
}

function stopStream() {
  connectProcess.value = false;
  connectionState.value = false;
  state.value = 0;
  ws.send(JSON.stringify({ type: 'close' }))
  ws.close();
}

function openStream() {
  open('http://streamsnipe.live??streamId=' + producerId.value);
}
</script>

<template>
  <div>
    <div v-if="!connectionState && !connectProcess">
      <button class="stream-button" @click="startStream">Stream</button>
    </div>
    <div v-if="state > 3 && connectionState">
      <button id="stop-stream" class="stream-button" @click="stopStream">Stop stream</button>
    </div>
    <div v-if="state < 3 && connectProcess" class="text-white">
      Connecting...
    </div>
    <div v-if="state > 3" class="text-white">
      Streaming... <a @click.prevent="openStream" :href="'http://streamsnipe.live??streamId=' + producerId">Open stream</a>
    </div>
  </div>
</template>

<style scoped>
.text-white {
  color: white;
}
.stream-button {
  border-radius: 9999px;
  background-color: #4c036c;
  font-size: 1.5rem;
  padding-left: 3rem;
  padding-right: 3rem;
  color: white;
}
.stream-button:hover {
  background-color: #5c037c;
  border: 1px solid #5c037c;
  font-size: 1.6rem;
}
#stop-stream {
  background-color: #6a0e01;
}
#stop-stream:hover {
  background-color: #7a0e01;
  border: 1px solid #7a0e01;
}
</style>