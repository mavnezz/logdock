import type { Pinia } from 'pinia'
import { createRouter, createWebHistory, type RouteRecordRaw, type Router } from 'vue-router'

import { useSessionStore } from '@/shared/session/store'
import { api, unwrap } from '@/shared/api/client'
import { remember, remembered } from '@/shared/lib/remembered'

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
      { path: '', redirect: '/overview' },
      { path: 'overview', component: () => import('@/features/overview/OverviewPage.vue') },
      { path: 'logs', component: () => import('@/features/logs/LogsPage.vue') },
      { path: 'issues', component: () => import('@/features/issues/IssuesPage.vue') },
      {
        path: 'issues/:id',
        component: () => import('@/features/issues/IssuePage.vue'),
        props: true,
      },
      {
        path: 'performance',
        component: () => import('@/features/performance/PerformancePage.vue'),
      },
      { path: 'queues', component: () => import('@/features/performance/QueuesPage.vue') },
      { path: 'releases', component: () => import('@/features/releases/ReleasesPage.vue') },
      {
        path: 'traces/:id',
        component: () => import('@/features/traces/TracePage.vue'),
        props: true,
      },
      { path: 'monitors', component: () => import('@/features/monitors/MonitorsPage.vue') },
      {
        path: 'monitors/:id',
        component: () => import('@/features/monitors/MonitorPage.vue'),
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
      { path: 'storage', component: () => import('@/features/system/StoragePage.vue') },
      { path: 'profile', component: () => import('@/features/auth/ProfilePage.vue') },
    ],
  },
  { path: '/:pathMatch(.*)*', redirect: '/' },
]

/** Sends visitors without a session to the setup or the sign-in. */
export function installGuard(router: Router, pinia: Pinia): void {
  router.beforeEach(async (to, from) => {
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
    // A list opened from elsewhere without filters gets those it was left
    // with; on the list itself, no filters means its defaults.
    if (Object.keys(to.query).length === 0 && from.path !== to.path) {
      const query = remembered(to.path)
      if (query) return { path: to.path, query, hash: to.hash, replace: true }
    }
    return true
  })
  router.afterEach((to) => remember(to.path, to.query))
}

export function createAppRouter(pinia: Pinia): Router {
  const router = createRouter({ history: createWebHistory(), routes })
  installGuard(router, pinia)
  return router
}
