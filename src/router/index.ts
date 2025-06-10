import {createRouter, createWebHashHistory, RouteRecordRaw} from "vue-router";
import { useUserStore } from "../stores/userStore";
import Start from "../Start.vue";
import Login from "../Login.vue";

const routes: RouteRecordRaw[] = [
    { path: "/", redirect: '/start' },
    { path: "/login", component: Login },
    { path: "/start", component: Start, meta: { requiresAuth: true } },
];

const router = createRouter({
    history: createWebHashHistory(),
    routes,
});

router.beforeEach((to, _from, next) => {
    const store = useUserStore();

    if (to.meta.requiresAuth && !store.isLoggedIn) {
        next('/login');
    } else {
        next();
    }
});

export default router;
