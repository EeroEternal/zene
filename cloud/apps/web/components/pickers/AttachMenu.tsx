"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import {
  COMPOSER_SKILLS,
  loadMcpServers,
  MAX_TURNS_PRESETS,
  maxTurnsLabel,
  PERMISSION_MODES,
  permissionLabel,
  saveMcpServers,
} from "@/lib/composerPrefs";
import { IconPaperclip, IconPlug, IconPlus, IconSkills } from "@/lib/icons";
import type { McpServer, PermissionMode } from "@/lib/types";
import { Menu, MenuItem, MenuSearch, MenuSep, MENU_FLYOUT, PromptDialog, Switch } from "../ui";

export type AttachSection = "files" | "skills" | "mcp" | "permission" | "maxTurns";

const ALL_SECTIONS: AttachSection[] = ["files", "skills", "mcp", "permission", "maxTurns"];

export function AttachMenu({
  open,
  onToggle,
  onClose,
  sections = ALL_SECTIONS,
  permissionMode,
  onSetPermissionMode,
  maxTurns,
  onSetMaxTurns,
  onInsertText,
  onFilesAttached,
  onNotice,
  compact,
}: {
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  sections?: AttachSection[];
  permissionMode?: PermissionMode;
  onSetPermissionMode?: (mode: PermissionMode) => void;
  maxTurns?: number;
  onSetMaxTurns?: (n: number) => void;
  onInsertText: (text: string) => void;
  onFilesAttached?: (names: string[]) => void;
  onNotice: (message: string, kind: "ok" | "error") => void;
  compact?: boolean;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [panel, setPanel] = useState<AttachSection | null>(null);
  const [mcpQuery, setMcpQuery] = useState("");
  const [mcpServers, setMcp] = useState<McpServer[]>([]);
  const [addMcpOpen, setAddMcpOpen] = useState(false);

  useEffect(() => {
    setMcp(loadMcpServers());
  }, []);

  const show = (id: AttachSection) => sections.includes(id);
  const filteredMcp = useMemo(() => {
    const q = mcpQuery.trim().toLowerCase();
    return mcpServers.filter((s) => !q || s.name.toLowerCase().includes(q));
  }, [mcpServers, mcpQuery]);

  const persistMcp = (next: McpServer[]) => {
    setMcp(next);
    saveMcpServers(next);
  };

  const attachFiles = (files: File[]) => {
    if (!files.length) return;
    const names = files.map((f) => f.name);
    onFilesAttached?.(names);
    onNotice(names.length === 1 ? `Attached ${names[0]}` : `Attached ${names.length} files`, "ok");
    onClose();
    setPanel(null);
  };

  const togglePanel = (id: AttachSection) => setPanel((cur) => (cur === id ? null : id));

  return (
    <div className="relative">
      <button
        type="button"
        className={
          compact
            ? `inline-flex h-6 w-6 items-center justify-center rounded-sm ${
                open ? "bg-active text-ink" : "bg-chip text-muted hover:bg-line-strong hover:text-ink"
              }`
            : `inline-flex h-7 w-7 items-center justify-center rounded-sm ${
                open ? "bg-active text-ink" : "bg-secondary text-muted hover:bg-active hover:text-ink"
              }`
        }
        title="Add"
        aria-label="Add"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          if (open) setPanel(null);
          onToggle();
        }}
      >
        <IconPlus className="h-3.5 w-3.5" />
      </button>
      {open && (
        <Menu
          className="absolute bottom-[calc(100%+8px)] left-0 z-[45] w-[200px] p-1.5"
          label="Add"
        >
          {show("files") && (
            <MenuItem
              icon={IconPaperclip}
              onClick={() => fileInputRef.current?.click()}
            >
              Files
            </MenuItem>
          )}
          {show("skills") && (
            <MenuItem
              icon={IconSkills}
              active={panel === "skills"}
              submenu
              onClick={() => togglePanel("skills")}
            >
              Skills
            </MenuItem>
          )}
          {show("mcp") && (
            <MenuItem
              icon={IconPlug}
              active={panel === "mcp"}
              submenu
              onClick={() => togglePanel("mcp")}
            >
              MCP Servers
            </MenuItem>
          )}
          {(show("permission") || show("maxTurns")) && <MenuSep />}
          {show("permission") && permissionMode && (
            <MenuItem
              active={panel === "permission"}
              hint={permissionLabel(permissionMode)}
              submenu
              onClick={() => togglePanel("permission")}
            >
              Permission
            </MenuItem>
          )}
          {show("maxTurns") && maxTurns != null && (
            <MenuItem
              active={panel === "maxTurns"}
              hint={maxTurnsLabel(maxTurns)}
              submenu
              onClick={() => togglePanel("maxTurns")}
            >
              Max turns
            </MenuItem>
          )}

          {panel === "skills" && (
            <div className={`${MENU_FLYOUT} w-[280px] max-[720px]:w-[min(280px,calc(100vw-48px))]`} role="menu">
              <div className="max-h-[260px] overflow-auto p-1.5">
                {COMPOSER_SKILLS.map((s) => (
                  <MenuItem
                    key={s.id}
                    onClick={() => {
                      onInsertText(s.insert);
                      onClose();
                      setPanel(null);
                    }}
                  >
                    {s.label}
                  </MenuItem>
                ))}
              </div>
            </div>
          )}

          {panel === "mcp" && (
            <div className={`${MENU_FLYOUT} w-[280px] max-[720px]:w-[min(280px,calc(100vw-48px))]`} role="menu">
              <MenuSearch value={mcpQuery} onChange={setMcpQuery} placeholder="Search MCP servers…" />
              <div className="max-h-[260px] overflow-auto p-1.5">
                {!filteredMcp.length ? (
                  <p className="m-0 p-2 text-xs text-muted">No MCP servers</p>
                ) : (
                  filteredMcp.map((s) => (
                    <div
                      key={s.id}
                      className="flex w-full items-center gap-2 rounded-lg p-2 text-left text-[13px] text-ink hover:bg-secondary"
                    >
                      <span className="relative grid h-[22px] w-[22px] shrink-0 place-items-center rounded-md bg-secondary">
                        <IconPlug className="h-3.5 w-3.5 text-muted" />
                        <span
                          className={`absolute -bottom-px -right-px h-[7px] w-[7px] rounded-full border-[1.5px] border-canvas ${
                            s.enabled ? "bg-ok" : "bg-[#C4C7C5]"
                          }`}
                        />
                      </span>
                      <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
                        {s.name}
                      </span>
                      {s.needsLogin && !s.enabled && (
                        <button
                          type="button"
                          className="h-[22px] rounded-md border border-line-strong bg-canvas px-2 text-[11px] font-medium text-ink hover:bg-secondary"
                          onClick={() => onNotice("MCP login coming soon", "ok")}
                        >
                          Login
                        </button>
                      )}
                      <Switch
                        checked={s.enabled}
                        label={`Toggle ${s.name}`}
                        onChange={(enabled) =>
                          persistMcp(
                            mcpServers.map((m) =>
                              m.id === s.id
                                ? { ...m, enabled, needsLogin: enabled ? false : m.needsLogin }
                                : m,
                            ),
                          )
                        }
                      />
                    </div>
                  ))
                )}
              </div>
              <div className="border-t border-line p-1.5">
                <MenuItem icon={IconPlus} onClick={() => setAddMcpOpen(true)}>
                  Add MCP
                </MenuItem>
              </div>
            </div>
          )}

          {panel === "permission" && permissionMode && onSetPermissionMode && (
            <div className={`${MENU_FLYOUT} w-[240px] max-[720px]:w-[min(240px,calc(100vw-48px))]`} role="menu">
              <div className="p-1.5">
                {PERMISSION_MODES.map((mode) => (
                  <MenuItem
                    key={mode.id}
                    checked={permissionMode === mode.id}
                    onClick={() => {
                      onSetPermissionMode(mode.id);
                      onClose();
                      setPanel(null);
                    }}
                  >
                    <span className="flex min-w-0 flex-col">
                      <span>{mode.label}</span>
                      <span className="text-[11px] font-normal text-muted">{mode.hint}</span>
                    </span>
                  </MenuItem>
                ))}
              </div>
            </div>
          )}

          {panel === "maxTurns" && maxTurns != null && onSetMaxTurns && (
            <div className={`${MENU_FLYOUT} w-[220px] max-[720px]:w-[min(220px,calc(100vw-48px))]`} role="menu">
              <div className="border-b border-line px-3 py-2">
                <p className="m-0 text-[11px] leading-snug text-muted">
                  Steps per turn before the agent pauses for a follow-up.
                </p>
              </div>
              <div className="p-1.5">
                {MAX_TURNS_PRESETS.map((preset) => (
                  <MenuItem
                    key={preset.label}
                    checked={maxTurns === preset.value}
                    onClick={() => {
                      onSetMaxTurns(preset.value);
                      onClose();
                      setPanel(null);
                    }}
                  >
                    {preset.label}
                  </MenuItem>
                ))}
              </div>
            </div>
          )}
        </Menu>
      )}
      <input
        ref={fileInputRef}
        className="sr-only"
        type="file"
        multiple
        tabIndex={-1}
        aria-hidden="true"
        onChange={(e) => {
          attachFiles(Array.from(e.target.files || []));
          e.target.value = "";
        }}
      />
      <PromptDialog
        open={addMcpOpen}
        title="Add MCP server"
        body="Give the server a short name. Connection details can be configured later."
        placeholder="Server name"
        confirmLabel="Add server"
        onCancel={() => setAddMcpOpen(false)}
        onConfirm={(name) => {
          persistMcp([...mcpServers, { id: "mcp-" + Date.now(), name, enabled: true, needsLogin: false }]);
          setAddMcpOpen(false);
          onNotice("MCP added", "ok");
        }}
      />
    </div>
  );
}
