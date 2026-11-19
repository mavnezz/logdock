import type { Pinia } from 'pinia'
import { createRouter, createWebHistory, type RouteRecordRaw, type Router } from 'vue-router'

import { useSessionStore } from '@/shared/session/store'
import { api, unwrap } from '@/shared/api/client'

declare module 'vue-router' {
  interface RouteMeta {
    /** Shown without a session. */
    public?: boolean
    /** For administrators only. */
    admin?: boolean
  }
}

export const routes: RouteRecordRaw[] = [
  {
    path: '/login',
    name: 'login',
    component: () => import('@/features/auth/LoginPage.vue'),
    meta: { public: true },
  },
  {
    path: '/setup',
    name: 'setup',
    component: () => import('@/features/auth/SetupPage.vue'),
    meta: { public: true },
  },
  {
    path: '/',
    component: () => import('./AppShell.vue'),
    children: [
      { path: '', redirect: '/logs' },
      { path: 'logs', component: () => import('@/features/logs/LogsPage.vue') },
      { path: 'issues', component: () => import('@/features/issues/IssuesPage.vue') },
      {
        path: 'issues/:id',
        component: () => import('@/features/issues/IssuePage.vue'),
        props: true,
      },
      { path: 'projects', component: () => import('@/features/projects/ProjectsPage.vue') },
      {
        path: 'projects/:id',
        component: () => import('@/features/projects/ProjectPage.vue'),
        props: true,
      },
      {
        path: 'accounts',
        component: () => import('@/features/accounts/AccountsPage.vue'),
        meta: { admin: true },
      },
      {
        path: 'audit',
        component: () => import('@/features/accounts/AuditPage.vue'),
        meta: { admin: true },
      },
      { path: 'profile', component: () => import('@/features/auth/ProfilePage.vue') },
    ],
  },
  { path: '/:pathMatch(.*)*', redirect: '/' },
]

/** Sends visitors without a session to the setup or the sign-in. */
export function installGuard(router: Router, pinia: Pinia): void {
  router.beforeEach(async (to) => {
    const store = useSessionStore(pinia)
    const session = await store.load().catch(() => null)
    if (to.meta.public) {
      return session && to.name === 'login' ? '/' : true
    }
    if (!session) {
      const setup = await unwrap(api.GET('/api/v1/setup')).catch(() => ({ needed: false }))
      if (setup.needed) return { name: 'setup' }
      return { name: 'login', query: to.fullPath === '/' ? {} : { next: to.fullPath } }
    }
    if (to.meta.admin && session.account.role !== 'admin') return '/'
    return true
  })
}

export function createAppRouter(pinia: Pinia): Router {
  const router = createRouter({ history: createWebHistory(), routes })
  installGuard(router, pinia)
  return router
}
