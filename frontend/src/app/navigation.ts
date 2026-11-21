// The sidebar: what there is to go to, in order. `section` starts a new
// group; `admin` shows an entry to administrators only; `key` goes there
// after `g` (g then l: the logs).

export interface NavItem {
  name: string
  path: string
  icon: string
  section?: string
  admin?: boolean
  key?: string
}

export const navigation: NavItem[] = [
  { name: 'overview', path: '/overview', icon: 'overview', section: 'observe', key: 'o' },
  { name: 'logs', path: '/logs', icon: 'logs', key: 'l' },
  { name: 'issues', path: '/issues', icon: 'issues', key: 'i' },
  { name: 'performance', path: '/performance', icon: 'performance', key: 'p' },
  { name: 'queues', path: '/queues', icon: 'queues', key: 'q' },
  { name: 'releases', path: '/releases', icon: 'releases', key: 'r' },
  { name: 'monitors', path: '/monitors', icon: 'monitors', key: 'm' },
  { name: 'projects', path: '/projects', icon: 'projects', section: 'setup', key: 'j' },
  { name: 'storage', path: '/storage', icon: 'storage', key: 's' },
  { name: 'accounts', path: '/accounts', icon: 'accounts', admin: true, key: 'a' },
  { name: 'audit', path: '/audit', icon: 'audit', admin: true },
  { name: 'profile', path: '/profile', icon: 'profile' },
]

/** The entry a path belongs to: its list, or the list of what it shows. */
export function navItemOf(path: string): NavItem | undefined {
  return navigation.find((item) => path === item.path || path.startsWith(`${item.path}/`))
}
