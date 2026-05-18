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

    await useUserStore().login(data.user.name, data.access_token);
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