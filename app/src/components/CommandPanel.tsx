import { useState } from "react";
import type { LineEnding } from "../bytes";
import { type Command, type CommandGroup, newCommand, newGroup } from "../commands";
import type { Strings } from "../i18n";

/** Progress of a run of one group. */
export interface RunProgress {
  groupId: string;
  /** 1-based number of the command being sent. */
  index: number;
  total: number;
}

interface Props {
  t: Strings;
  groups: CommandGroup[];
  onChange: (groups: CommandGroup[]) => void;
  /** Whether commands can be sent (the selected session is open). */
  enabled: boolean;
  onSend: (command: Command) => void;
  run: RunProgress | null;
  onRun: (group: CommandGroup) => void;
  onStop: () => void;
  /** Line ending given to new commands. */
  defaultEnding: LineEnding;
}

/**
 * Saved commands beside the log, in named groups. Clicking a command sends
 * it; "Run all" sends the group's commands in order with each one's delay.
 * Deleting asks for a second click instead of a dialog.
 */
export function CommandPanel({
  t,
  groups,
  onChange,
  enabled,
  onSend,
  run,
  onRun,
  onStop,
  defaultEnding,
}: Props) {
  const [groupId, setGroupId] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [editingGroup, setEditingGroup] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);

  const group = groups.find((g) => g.id === groupId) ?? groups[0];
  if (!group) return null;

  const setGroup = (next: CommandGroup) =>
    onChange(groups.map((g) => (g.id === next.id ? next : g)));
  const setCommand = (id: string, change: Partial<Command>) =>
    setGroup({
      ...group,
      commands: group.commands.map((c) => (c.id === id ? { ...c, ...change } : c)),
    });
  const move = (index: number, delta: number) => {
    const commands = [...group.commands];
    const [c] = commands.splice(index, 1);
    if (!c) return;
    commands.splice(index + delta, 0, c);
    setGroup({ ...group, commands });
  };
  // A second click on the same delete button deletes.
  const confirmed = (key: string) => {
    if (confirm === key) {
      setConfirm(null);
      return true;
    }
    setConfirm(key);
    return false;
  };

  const addGroup = () => {
    const g = newGroup(t.newGroupName(groups.length + 1));
    onChange([...groups, g]);
    setGroupId(g.id);
    setEditingGroup(true);
  };
  const deleteGroup = () => {
    if (!confirmed(`group:${group.id}`)) return;
    const rest = groups.filter((g) => g.id !== group.id);
    onChange(rest.length > 0 ? rest : [newGroup(t.defaultGroup)]);
    setGroupId(null);
    setEditingGroup(false);
  };
  const addCommand = () => {
    const c = newCommand(t.newCommandName(group.commands.length + 1), defaultEnding);
    setGroup({ ...group, commands: [...group.commands, c] });
    setEditing(c.id);
  };

  const running = run !== null && run.groupId === group.id;

  return (
    <aside className="command-panel" aria-label={t.commands}>
      <div className="command-head">
        {editingGroup ? (
          <input
            aria-label={t.groupName}
            value={group.name}
            onChange={(e) => setGroup({ ...group, name: e.target.value })}
            onKeyDown={(e) => e.key === "Enter" && setEditingGroup(false)}
          />
        ) : (
          <select
            aria-label={t.group}
            value={group.id}
            onChange={(e) => {
              setGroupId(e.target.value);
              setEditing(null);
              setConfirm(null);
            }}
          >
            {groups.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
        )}
        <button
          type="button"
          className="chip"
          aria-pressed={editingGroup}
          onClick={() => {
            setEditingGroup((e) => !e);
            setConfirm(null);
          }}
        >
          {editingGroup ? t.done : t.edit}
        </button>
        {editingGroup ? (
          <button type="button" className="chip danger" onClick={deleteGroup}>
            {confirm === `group:${group.id}` ? t.confirmDelete : t.deleteGroup}
          </button>
        ) : (
          <button type="button" className="chip" title={t.newGroup} onClick={addGroup}>
            +
          </button>
        )}
      </div>
      <ul className="command-list">
        {group.commands.length === 0 && <li className="muted">{t.noCommands}</li>}
        {group.commands.map((c, i) => (
          <li key={c.id} className={running && run.index === i + 1 ? "command current" : "command"}>
            {editing === c.id ? (
              <div className="command-edit">
                <input
                  aria-label={t.commandName}
                  autoFocus
                  value={c.name}
                  onChange={(e) => setCommand(c.id, { name: e.target.value })}
                />
                <textarea
                  aria-label={t.commandText}
                  rows={2}
                  value={c.text}
                  placeholder={c.hex ? "01 a0 ff" : "AT+GMR"}
                  onChange={(e) => setCommand(c.id, { text: e.target.value })}
                />
                <div className="command-row">
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={c.hex}
                      onChange={(e) => setCommand(c.id, { hex: e.target.checked })}
                    />
                    {t.hexInput}
                  </label>
                  <select
                    aria-label={t.lineEnding}
                    value={c.ending}
                    disabled={c.hex}
                    onChange={(e) => setCommand(c.id, { ending: e.target.value as LineEnding })}
                  >
                    <option value="none">{t.endingNone}</option>
                    <option value="lf">\n</option>
                    <option value="crlf">\r\n</option>
                    <option value="cr">\r</option>
                  </select>
                  <label className="check" title={t.delayAfterHint}>
                    {t.delayAfter}
                    <input
                      aria-label={t.delayAfter}
                      className="delay"
                      type="number"
                      min={0}
                      max={86_400_000}
                      value={c.delayMs}
                      onChange={(e) => {
                        const ms = Math.round(Number(e.target.value));
                        if (Number.isFinite(ms)) {
                          setCommand(c.id, { delayMs: Math.min(Math.max(ms, 0), 86_400_000) });
                        }
                      }}
                    />
                  </label>
                </div>
                <div className="command-row">
                  <button
                    type="button"
                    className="chip"
                    disabled={i === 0}
                    onClick={() => move(i, -1)}
                  >
                    {t.moveUp}
                  </button>
                  <button
                    type="button"
                    className="chip"
                    disabled={i === group.commands.length - 1}
                    onClick={() => move(i, 1)}
                  >
                    {t.moveDown}
                  </button>
                  <button
                    type="button"
                    className="chip danger"
                    onClick={() => {
                      if (!confirmed(`command:${c.id}`)) return;
                      setGroup({ ...group, commands: group.commands.filter((x) => x.id !== c.id) });
                      setEditing(null);
                    }}
                  >
                    {confirm === `command:${c.id}` ? t.confirmDelete : t.deleteCommand}
                  </button>
                  <span className="spacer" />
                  <button type="button" className="chip" onClick={() => setEditing(null)}>
                    {t.done}
                  </button>
                </div>
              </div>
            ) : (
              <>
                <button
                  type="button"
                  className="command-send"
                  title={c.text}
                  aria-label={t.sendCommand(c.name)}
                  disabled={!enabled || running}
                  onClick={() => onSend(c)}
                >
                  <span className="command-name">{c.name || "—"}</span>
                  <span className="command-text">{c.hex ? `hex ${c.text}` : c.text}</span>
                </button>
                <button
                  type="button"
                  className="chip"
                  aria-label={t.editCommand(c.name)}
                  onClick={() => {
                    setEditing(c.id);
                    setConfirm(null);
                  }}
                >
                  {t.edit}
                </button>
              </>
            )}
          </li>
        ))}
      </ul>
      <div className="command-foot">
        <button type="button" className="chip" onClick={addCommand}>
          + {t.addCommand}
        </button>
        <span className="spacer" />
        {running ? (
          <button type="button" className="chip danger" onClick={onStop}>
            {t.stopRun} {run.index}/{run.total}
          </button>
        ) : (
          <button
            type="button"
            className="chip"
            disabled={!enabled || run !== null || group.commands.length === 0}
            title={t.runAllHint}
            onClick={() => onRun(group)}
          >
            ▶ {t.runAll}
          </button>
        )}
      </div>
    </aside>
  );
}
