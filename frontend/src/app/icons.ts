import {
  Activity,
  Bug,
  CalendarClock,
  FolderKanban,
  HardDrive,
  Layers,
  LayoutDashboard,
  type LucideIcon,
  ScrollText,
  ShieldCheck,
  Tag,
  UserRound,
  Users,
} from 'lucide-vue-next'

/** Each navigation entry's icon, by its `icon`. */
export const NAV_ICONS: Record<string, LucideIcon> = {
  projects: FolderKanban,
  storage: HardDrive,
  accounts: Users,
  audit: ShieldCheck,
  profile: UserRound,
  overview: LayoutDashboard,
  logs: ScrollText,
  issues: Bug,
  performance: Activity,
  queues: Layers,
  releases: Tag,
  monitors: CalendarClock,
}
