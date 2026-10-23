import { useState, type ReactNode } from "react";

/**
 * A small Markdown renderer for chat replies. It builds React elements directly
 * (never raw HTML), so a reply can't inject markup or scripts. Covers what models
 * actually write: headings, paragraphs, lists (nested by indent), task boxes, code
 * blocks, quotes, tables, rules, and inline code / bold / italic / strike / links.
 * Unfinished syntax mid-stream (an open ``` fence, say) renders sensibly.
 */
export function Markdown({ text }: { text: string }) {
  return <div className="md flex flex-col gap-3 leading-relaxed">{blocks(text.replace(/\r\n?/g, "\n"))}</div>;
}

// ---------- blocks ----------

// Every block pattern runs on each streamed token, so none may backtrack badly:
// no two adjacent whitespace runs, and trailing spaces are trimmed before testing.
const FENCE = /^( {0,3})(`{3,}|~{3,})[ \t]*([\w+#.-]*)$/;
const HEADING = /^(#{1,6})[ \t]+(.*)$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const LIST_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const TABLE_SEP = /^\|?(?:[ \t]*:?-+:?[ \t]*\|)*[ \t]*:?-+:?[ \t]*\|?$/;

function blocks(text: string): ReactNode[] {
  const lines = text.split("\n");
  const out: ReactNode[] = [];
  let i = 0;
  let key = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (!line.trim()) {
      i++;
      continue;
    }

    const fence = FENCE.exec(line.trimEnd());
    if (fence) {
      const marker = fence[2];
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(marker)) body.push(lines[i++]);
      i++; // closing fence (or end of a reply still streaming)
      out.push(<CodeBlock key={key++} lang={fence[3]} code={body.join("\n")} />);
      continue;
    }

    const heading = HEADING.exec(line.trimEnd());
    if (heading) {
      const level = heading[1].length;
      // Closing #s ("## Title ##") are decoration.
      const title = heading[2].replace(/[ \t]#+$/, "").trimEnd();
      const size = ["text-xl", "text-lg", "text-base", "text-base", "text-sm", "text-sm"][level - 1];
      const Tag = `h${Math.min(level + 1, 6)}` as "h2";
      out.push(
        <Tag key={key++} className={`${size} mt-2 font-semibold text-ink`}>
          {inline(title)}
        </Tag>,
      );
      i++;
      continue;
    }

    if (RULE.test(line)) {
      out.push(<hr key={key++} className="border-line" />);
      i++;
      continue;
    }

    if (QUOTE.test(line)) {
      const body: string[] = [];
      while (i < lines.length && QUOTE.test(lines[i])) body.push(QUOTE.exec(lines[i++])![1]);
      out.push(
        <blockquote key={key++} className="flex flex-col gap-2 border-l-2 border-ember/60 pl-4 text-soft">
          {blocks(body.join("\n"))}
        </blockquote>,
      );
      continue;
    }

    if (line.includes("|") && i + 1 < lines.length && TABLE_SEP.test(lines[i + 1].trim()) && lines[i + 1].includes("-")) {
      const head = cells(line);
      const align = cells(lines[i + 1]).map((c) =>
        c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : "left",
      );
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && lines[i].includes("|") && lines[i].trim()) rows.push(cells(lines[i++]));
      out.push(
        <div key={key++} className="overflow-x-auto rounded-xl border border-line">
          <table className="w-full border-collapse text-sm">
            <thead className="bg-raised">
              <tr>
                {head.map((h, c) => (
                  <th key={c} style={{ textAlign: align[c] as "left" }} className="px-3 py-2 font-semibold">
                    {inline(h)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, n) => (
                <tr key={n} className="border-t border-line">
                  {head.map((_, c) => (
                    <td key={c} style={{ textAlign: align[c] as "left" }} className="px-3 py-2 align-top">
                      {inline(r[c] ?? "")}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    if (LIST_ITEM.test(line)) {
      const start = i;
      const first = LIST_ITEM.exec(line)!;
      const ordered = /\d/.test(first[2]);
      // A top-level item of the other kind (bullets vs numbers) starts a new list.
      const otherKind = (l: string) => {
        const m = LIST_ITEM.exec(l);
        return !!m && m[1].length <= first[1].length && /\d/.test(m[2]) !== ordered;
      };
      while (
        i < lines.length &&
        !(i > start && otherKind(lines[i])) &&
        !(!lines[i].trim() && i + 1 < lines.length && otherKind(lines[i + 1])) &&
        (LIST_ITEM.test(lines[i]) ||
          (lines[i].trim() && /^\s{2,}/.test(lines[i])) ||
          (!lines[i].trim() && i + 1 < lines.length && (LIST_ITEM.test(lines[i + 1]) || /^\s{2,}\S/.test(lines[i + 1]))))
      )
        i++;
      out.push(<List key={key++} lines={lines.slice(start, i)} />);
      continue;
    }

    // Paragraph: until a blank line or the start of another block.
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !FENCE.test(lines[i].trimEnd()) &&
      !HEADING.test(lines[i].trimEnd()) &&
      !RULE.test(lines[i]) &&
      !QUOTE.test(lines[i]) &&
      !(para.length > 0 && LIST_ITEM.test(lines[i]))
    )
      para.push(lines[i++]);
    out.push(
      <p key={key++} className="whitespace-pre-wrap">
        {inline(para.join("\n"))}
      </p>,
    );
  }
  return out;
}

function cells(row: string): string[] {
  let r = row.trim();
  if (r.startsWith("|")) r = r.slice(1);
  if (r.endsWith("|") && !r.endsWith("\\|")) r = r.slice(0, -1);
  return r.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, "|"));
}

function List({ lines }: { lines: string[] }) {
  const first = LIST_ITEM.exec(lines[0])!;
  const indent = first[1].length;
  const ordered = /\d/.test(first[2]);
  const items: { text: string[]; start: number }[] = [];
  for (const line of lines) {
    const m = LIST_ITEM.exec(line);
    if (m && m[1].length <= indent) items.push({ text: [m[3]], start: parseInt(m[2], 10) });
    else if (items.length) items[items.length - 1].text.push(line.slice(Math.min(indent + 2, line.search(/\S|$/))));
  }
  const Tag = ordered ? "ol" : "ul";
  return (
    <Tag
      start={ordered ? items[0]?.start : undefined}
      className={`flex flex-col gap-1.5 pl-6 ${ordered ? "list-decimal" : "list-disc"} marker:text-muted`}
    >
      {items.map((item, n) => {
        const [head, ...rest] = item.text;
        const task = /^\[([ xX])\]\s+(.*)$/.exec(head);
        return (
          <li key={n} className={task ? "list-none -ml-6" : undefined}>
            {task ? (
              <span className="flex items-start gap-2">
                <input type="checkbox" checked={task[1] !== " "} readOnly disabled className="mt-1.5 accent-ember" />
                <span>{inline(task[2])}</span>
              </span>
            ) : (
              inline(head)
            )}
            {rest.some((l) => l.trim()) && <div className="mt-1.5 flex flex-col gap-2">{blocks(rest.join("\n"))}</div>}
          </li>
        );
      })}
    </Tag>
  );
}

function CodeBlock({ lang, code }: { lang: string; code: string }) {
  const [copied, setCopied] = useState(false);
  function copy() {
    void navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  }
  return (
    <div className="overflow-hidden rounded-xl border border-line bg-deep">
      <div className="flex items-center justify-between border-b border-line px-3 py-1.5 text-xs text-muted">
        <span className="font-mono">{lang || "code"}</span>
        <button type="button" onClick={copy} className="rounded px-2 py-0.5 hover:bg-raised hover:text-ink">
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="overflow-x-auto p-3.5 font-mono text-[13px] leading-relaxed text-soft">
        <code>{code}</code>
      </pre>
    </div>
  );
}

// ---------- inline ----------

/**
 * Inline syntax, most specific first. Code spans are matched before anything else.
 * Every span is length-bounded so odd input (thousands of `*` or `[`) stays fast
 * while a reply streams in and re-renders on every token.
 */
const INLINE =
  /(`{1,16})([^`][\s\S]{0,2000}?)\1(?!`)|!\[([^\]]{0,500})\]\(([^)\s]{1,2000})(?:\s[^)]{0,500})?\)|\[([^\]]{1,500})\]\(([^)\s]{1,2000})(?:\s[^)]{0,500})?\)|<(https?:\/\/[^>\s]{1,2000})>|(\*\*|__)(?=\S)([\s\S]{0,1000}?\S)\8|(~~)(?=\S)([\s\S]{0,1000}?\S)~~|(\*|_)(?=\S)([\s\S]{0,1000}?\S)\12(?![\w*])|(https?:\/\/[^\s<>()]{0,2000}[^\s<>().,;:!?'"])|\\([\\`*_{}[\]()#+\-.!|~>])/g;

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let key = 0;
  // A fresh regex per call: inline() recurses, and a shared global regex would
  // have its position reset by the inner call (an endless loop).
  const re = new RegExp(INLINE.source, "g");
  for (let m = re.exec(text); m; m = re.exec(text)) {
    // `_` inside words (snake_case) isn't emphasis.
    if (m[12] === "_" && m.index > 0 && /\w/.test(text[m.index - 1])) continue;
    if (m.index > last) out.push(text.slice(last, m.index));
    last = m.index + m[0].length;
    if (m[1]) {
      out.push(
        <code key={key++} className="rounded bg-raised px-1.5 py-0.5 font-mono text-[0.9em] text-soft">
          {m[2].replace(/^ (.*) $/, "$1")}
        </code>,
      );
    } else if (m[4] !== undefined) {
      // Images from replies aren't loaded (they could fetch from anywhere); show the link.
      out.push(<SafeLink key={key++} href={m[4]}>{m[3] || m[4]}</SafeLink>);
    } else if (m[5] !== undefined) {
      out.push(<SafeLink key={key++} href={m[6]}>{inline(m[5])}</SafeLink>);
    } else if (m[7]) {
      out.push(<SafeLink key={key++} href={m[7]}>{m[7]}</SafeLink>);
    } else if (m[8]) {
      out.push(
        <strong key={key++} className="font-semibold text-ink">
          {inline(m[9])}
        </strong>,
      );
    } else if (m[10]) {
      out.push(<del key={key++}>{inline(m[11])}</del>);
    } else if (m[12]) {
      out.push(<em key={key++}>{inline(m[13])}</em>);
    } else if (m[14]) {
      out.push(<SafeLink key={key++} href={m[14]}>{m[14]}</SafeLink>);
    } else if (m[15]) {
      out.push(m[15]);
    }
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Only http(s) and mailto links become clickable; they open outside the app. */
function SafeLink({ href, children }: { href: string; children: ReactNode }) {
  if (!/^(https?:|mailto:)/i.test(href)) return <span className="text-soft">{children}</span>;
  return (
    <a href={href} target="_blank" rel="noreferrer noopener" className="text-ember underline underline-offset-2 hover:text-ember/80">
      {children}
    </a>
  );
}
