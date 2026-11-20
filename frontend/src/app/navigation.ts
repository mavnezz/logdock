// The sidebar: what there is to go to, in order. `section` starts a new
// group; `admin` shows an entry to administrators only.

export interface NavItem {
  name: string
  path: string
  icon: string
  section?: string
  admin?: boolean
}

export const navigation: NavItem[] = [
  { name: 'logs', path: '/logs', icon: 'logs', section: 'observe' },
  { name: 'issues', path: '/issues', icon: 'issues' },
  { name: 'performance', path: '/performance', icon: 'performance' },
  { name: 'queues', path: '/queues', icon: 'queues' },
  { name: 'monitors', path: '/monitors', icon: 'monitors' },
  { name: 'projects', path: '/projects', icon: 'projects', section: 'setup' },
  { name: 'accounts', path: '/accounts', icon: 'accounts', admin: true },
  { name: 'audit', path: '/audit', icon: 'audit', admin: true },
  { name: 'profile', path: '/profile', icon: 'profile' },
]
