<script setup lang="ts">
import { ref } from "vue";
import { useRouter } from "vue-router";
import { useHttp } from "./composables/useHttp.ts";
import { useUserStore } from "./stores/userStore.ts";

const router = useRouter()
const email = ref('');
const password = ref('');
const errorMessage = ref('');

async function loginAction() {
  if (!email.value || !password.value) {
    return;
  }

  await useHttp(
      'POST',
      'login',
      { email: email.value, password: password.value }
  ).then(async (response) => {
    const data = await response.json();

    if (data.errors) {
      for (const error in data.errors) {
        errorMessage.value += data.errors[error][0] + '\n'
      }

      return;
    }

    useUserStore().login(data.user.name, data.access_token);
    await router.replace('/start')
  }).catch(err => {
    errorMessage.value = err.message;
  });
}
</script>

<template>
  <div>
    <div class="row logo-row">
      <img id="login-logo" alt="logo" src="./assets/logo.jpg"/>
    </div>
    <div class="row">
      <form @submit.prevent="loginAction">
        <div>
          <input class="app-input" v-model="email" placeholder="Login" />
        </div>
        <div>
          <input class="app-input" v-model="password" placeholder="Password" type="password"/>
        </div>
        <button type="submit">Login</button>
      </form>
    </div>
    <p>{{ errorMessage }}</p>
  </div>
</template>

<style scoped>
#login-logo {
  height: 100px;
}
.logo-row {
  margin-bottom: 1rem;
}
</style>