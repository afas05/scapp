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
  errorMessage.value = '';

  await useHttp(
      'POST',
      'login',
      { email: email.value, password: password.value }
  ).then(async (response) => {
    // Version below the server minimum; useHttp has moved to /update-required.
    if (response.status === 426) return;
    let data: any = null;
    try {
      data = await response.json();
    } catch { /* non-JSON body */ }

    if (data?.errors) {
      errorMessage.value = Object.values(data.errors)
          .map((messages: any) => messages[0])
          .join('\n');
      return;
    }

    // Any other failure (419, 429, 500, ...) has no `errors`; don't treat it as a login.
    if (!response.ok || !data?.user || !data?.access_token) {
      console.warn('[login] unexpected response', response.status, data);
      errorMessage.value = data?.message || `Login failed (${response.status})`;
      return;
    }

    const userStore = useUserStore();
    await userStore.login(data.user.name, data.access_token, data.user.id);
    // Login response has no plan; fetch it so the first recording gates correctly.
    await userStore.refreshPlan();
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
        <div class="submit-row">
          <button type="submit">Login</button>
        </div>
      </form>
    </div>
    <p>{{ errorMessage }}</p>
  </div>
</template>

<style scoped>
.row {
  display: flex;
  justify-content: center;
}
#login-logo {
  height: 100px;
}
.logo-row {
  margin-top: 8vh;
  margin-bottom: 1rem;
}
.submit-row {
  display: flex;
  justify-content: center;
}
button[type='submit'] {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 9px 20px;
  border-radius: 6px;
  background: linear-gradient(180deg, var(--accent) 0%, var(--accent-soft) 100%);
  color: #fff;
  border: 1px solid var(--accent);
  font-size: 13px;
  font-weight: 600;
  margin-top: 6px;
  box-shadow: 0 4px 20px var(--accent-glow);
}
p {
  text-align: center;
  color: #ff6b6b;
  font-size: 12px;
}
</style>