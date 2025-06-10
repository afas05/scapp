import { defineStore } from 'pinia'

export const useUserStore = defineStore('user', {
    state: () => ({
        name: '',
        sessionHash: '',
        isLoggedIn: false,
    }),
    actions: {
        login(name: string, sessionHash: string) {
            this.name = name
            this.sessionHash = sessionHash
            this.isLoggedIn = true;
        },
        logout() {
            this.name = ''
            this.sessionHash = ''
            this.isLoggedIn = false;
        }
    }
})
