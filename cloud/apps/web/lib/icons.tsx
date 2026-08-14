import {
  Archive,
  ArrowUp,
  Check,
  Copy,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  ChevronsDownUp,
  ChevronsUpDown,
  Code2,
  Cpu,
  Ellipsis,
  ExternalLink,
  Eye,
  File,
  FileDiff,
  Folder,
  FolderGit2,
  FolderOpen,
  GitBranch,
  HelpCircle,
  History,
  Home,
  LayoutDashboard,
  LayoutList,
  ListFilter,
  Loader2,
  LogOut,
  Maximize2,
  MessageSquare,
  Minimize2,
  PanelLeft,
  PanelLeftClose,
  PanelRight,
  PanelRightClose,
  Paperclip,
  Pencil,
  Plug,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Shield,
  Sparkles,
  Square,
  SquarePen,
  ThumbsDown,
  ThumbsUp,
  Trash2,
  User,
  type LucideIcon,
  type LucideProps,
} from "lucide-react";
import type { SVGProps } from "react";

/** Lucide defaults tuned for crisp rendering at 14–16px in Console UI. */
const ICON_DEFAULTS = {
  strokeWidth: 2,
  absoluteStrokeWidth: true,
} satisfies Pick<LucideProps, "strokeWidth" | "absoluteStrokeWidth">;

function lucideIcon(Icon: LucideIcon) {
  const Wrapped = ({ strokeWidth, absoluteStrokeWidth, ...props }: LucideProps) => (
    <Icon
      strokeWidth={strokeWidth ?? ICON_DEFAULTS.strokeWidth}
      absoluteStrokeWidth={absoluteStrokeWidth ?? ICON_DEFAULTS.absoluteStrokeWidth}
      aria-hidden={props["aria-hidden"] ?? true}
      {...props}
    />
  );
  Wrapped.displayName = Icon.displayName ?? Icon.name;
  return Wrapped;
}

/** Lucide omits brand marks — use official SVG paths (see public/icons/* and AGENTS.md). */
export const IconGithub = (props: SVGProps<SVGSVGElement>) => (
  <svg viewBox="0 0 32 32" fill="currentColor" aria-hidden="true" {...props}>
    <path
      fillRule="evenodd"
      clipRule="evenodd"
      d="M16 0C7.16 0 0 7.16 0 16C0 23.08 4.58 29.06 10.94 31.18C11.74 31.32 12.04 30.84 12.04 30.42C12.04 30.04 12.02 28.78 12.02 27.44C8 28.18 6.96 26.46 6.64 25.56C6.46 25.1 5.68 23.68 5 23.3C4.44 23 3.64 22.26 4.98 22.24C6.24 22.22 7.14 23.4 7.44 23.88C8.88 26.3 11.18 25.62 12.1 25.2C12.24 24.16 12.66 23.46 13.12 23.06C9.56 22.66 5.84 21.28 5.84 15.16C5.84 13.42 6.46 11.98 7.48 10.86C7.32 10.46 6.76 8.82 7.64 6.62C7.64 6.62 8.98 6.2 12.04 8.26C13.32 7.9 14.68 7.72 16.04 7.72C17.4 7.72 18.76 7.9 20.04 8.26C23.1 6.18 24.44 6.62 24.44 6.62C25.32 8.82 24.76 10.46 24.6 10.86C25.62 11.98 26.24 13.4 26.24 15.16C26.24 21.3 22.5 22.66 18.94 23.06C19.52 23.56 20.02 24.52 20.02 26.02C20.02 28.16 20 29.88 20 30.42C20 30.84 20.3 31.34 21.1 31.18C27.42 29.06 32 23.06 32 16C32 7.16 24.84 0 16 0Z"
    />
  </svg>
);

export const IconGitlab = (props: SVGProps<SVGSVGElement>) => (
  <svg viewBox="0 0 500 500" fill="currentColor" aria-hidden="true" {...props}>
    <path d="M249.9,476.8 249.9,476.8 340.6,197.7 159.2,197.7 249.9,476.8z" />
    <path d="M32.1,197.7 32.1,197.7 4.5,282.5c-2.5,7.7.2,16.2 6.8,21l238.5,173.3L32.1,197.7z" />
    <path d="M32.1,197.7h127.1L104.6,29.6c-2.8-8.6-15-8.6-17.9,0L32.1,197.7z" />
    <path d="M467.6,197.7 467.6,197.7 495.2,282.5c2.5,7.7-.2,16.2-6.8,21L249.9,476.8 467.6,197.7z" />
    <path d="M467.6,197.7H340.5l54.6-168.1c2.8-8.6,15-8.6,17.9,0L467.6,197.7z" />
  </svg>
);

export const IconArchive = lucideIcon(Archive);
export const IconArrowUp = lucideIcon(ArrowUp);
export const IconBranch = lucideIcon(GitBranch);
export const IconCheck = lucideIcon(Check);
export const IconCopy = lucideIcon(Copy);
export const IconCode = lucideIcon(Code2);
export const IconChevronDown = lucideIcon(ChevronDown);
export const IconChevronRight = lucideIcon(ChevronRight);
export const IconChevronUp = lucideIcon(ChevronUp);
export const IconChevronsCollapse = lucideIcon(ChevronsDownUp);
export const IconChevronsExpand = lucideIcon(ChevronsUpDown);
export const IconCpu = lucideIcon(Cpu);
export const IconDots = lucideIcon(Ellipsis);
export const IconExternal = lucideIcon(ExternalLink);
export const IconEye = lucideIcon(Eye);
export const IconFile = lucideIcon(File);
export const IconFileDiff = lucideIcon(FileDiff);
export const IconFilter = lucideIcon(ListFilter);
export const IconFolder = lucideIcon(Folder);
export const IconFolderOpen = lucideIcon(FolderOpen);
export const IconHelp = lucideIcon(HelpCircle);
export const IconHistory = lucideIcon(History);
export const IconHome = lucideIcon(Home);
export const IconLayoutDashboard = lucideIcon(LayoutDashboard);
export const IconLayoutList = lucideIcon(LayoutList);
export const IconLoader = lucideIcon(Loader2);
export const IconLogout = lucideIcon(LogOut);
export const IconMaximize = lucideIcon(Maximize2);
export const IconMessage = lucideIcon(MessageSquare);
export const IconMinimize = lucideIcon(Minimize2);
export const IconPanelLeft = lucideIcon(PanelLeft);
export const IconPanelLeftClose = lucideIcon(PanelLeftClose);
export const IconPanelRight = lucideIcon(PanelRight);
export const IconPanelRightClose = lucideIcon(PanelRightClose);
export const IconPaperclip = lucideIcon(Paperclip);
export const IconPencil = lucideIcon(Pencil);
export const IconPlug = lucideIcon(Plug);
export const IconPlus = lucideIcon(Plus);
export const IconRefresh = lucideIcon(RefreshCw);
export const IconRepo = lucideIcon(FolderGit2);
export const IconSearch = lucideIcon(Search);
export const IconSettings = lucideIcon(Settings);
export const IconShield = lucideIcon(Shield);
export const IconSkills = lucideIcon(Sparkles);
export const IconStop = lucideIcon(Square);
export const IconSquarePen = lucideIcon(SquarePen);
export const IconThumbsDown = lucideIcon(ThumbsDown);
export const IconThumbsUp = lucideIcon(ThumbsUp);
export const IconTrash = lucideIcon(Trash2);
export const IconUser = lucideIcon(User);
