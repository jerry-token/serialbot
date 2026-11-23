// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useEffect, useRef } from "react";
import type { ApprovalRequest, Decision } from "../api";
import type { Strings } from "../i18n";

interface Props {
  t: Strings;
  /** Waiting requests, oldest first; the dialog shows the first one. */
  queue: ApprovalRequest[];
  onAnswer: (id: number, decision: Decision) => void;
}

/**
 * Asks the user about the oldest action an AI agent is waiting to carry
 * out. Escape denies it, and "Deny" has the focus, so a key pressed while
 * typing elsewhere never allows anything. Renders nothing while the queue
 * is empty.
 */
export function ApprovalDialog({ t, queue, onAnswer }: Props) {
  const request = queue[0];
  const deny = useRef<HTMLButtonElement>(null);
  const id = request?.id;
  useEffect(() => {
    if (id !== undefined) deny.current?.focus();
  }, [id]);
  if (!request) return null;
  const answer = (decision: Decision) => onAnswer(request.id, decision);
  return (
    <div className="approval-backdrop">
      <div
        className="approval card"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="approval-title"
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            answer("deny");
          }
        }}
      >
        <h2 id="approval-title">{t.approvalTitle}</h2>
        <p>
          <code className="approval-tool">{request.tool}</code>{" "}
          {request.session === null ? t.approvalOpen(request.port) : t.approvalOn(request.port)}
        </p>
        {request.detail && <pre className="approval-detail">{request.detail}</pre>}
        <p className="muted">
          {t.approvalHint}
          {queue.length > 1 && ` ${t.approvalQueue(queue.length - 1)}`}
        </p>
        <div className="row approval-actions">
          <button type="button" className="primary" onClick={() => answer("once")}>
            {t.allowOnce}
          </button>
          {request.session !== null && (
            <button type="button" onClick={() => answer("session")}>
              {t.allowSession}
            </button>
          )}
          <button type="button" ref={deny} onClick={() => answer("deny")}>
            {t.deny}
          </button>
        </div>
      </div>
    </div>
  );
}
