import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'

export const routes: RouteRecordRaw[] = [
  { path: '/', name: 'welcome', component: () => import('./WelcomePage.vue') },
  { path: '/:pathMatch(.*)*', redirect: '/' },
]

export function createAppRouter() {
  return createRouter({ history: createWebHistory(), routes })
}
