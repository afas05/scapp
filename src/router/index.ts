import {createRouter, createWebHashHistory, RouteRecordRaw} from "vue-router";
import { useUserStore } from "../stores/userStore";
import Start from "../Start.vue";
import Login from "../Login.vue";
import Editor from "../Editor.vue";
import UpdateRequired from "../UpdateRequired.vue";
import { updateRequired } from "../lib/updater";

const routes: RouteRecordRaw[] = [
    { path: "/", redirect: '/start' },
    { path: "/login", component: Login },
    { path: "/update-required", component: UpdateRequired },
    { path: "/start", component: Start, meta: { requiresAuth: true } },
    { path: "/editor", component: Editor, meta: { requiresAuth: true } },
];

const router = createRouter({
    history: createWebHashHistory(),
    routes,
});

router.beforeEach((to, _from, next) => {
    const store = useUserStore();

    // Once the server has refused this version, nothing else is reachable.
    if (updateRequired.value && to.path !== '/update-required') {
        next('/update-required');
    } else if (to.meta.requiresAuth && !store.isLoggedIn) {
        next('/login');
    } else {
        next();
    }
});

export default router;
