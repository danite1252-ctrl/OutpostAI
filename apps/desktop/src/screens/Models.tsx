import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import {
  api,
  ApiError,
  type Badge,
  type CatalogEntry,
  type Category,
  type EngineInfo,
  type Fit,
  type Hardware,
  type InstalledModel,
  type ModelsOverview,
  type PullState,
} from "../api";
import { EngineCard } from "../components/engines";
import { Button, ErrorNote } from "../components/ui";
import { downloadsChanged, formatEta, formatSpeed } from "../components/downloads";

const BADGE_LABEL: Record<Badge, string> = {
  vision: "Vision",
  tools: "Tools / Agent",
  thinking: "Thinking",
  uncensored: "Uncensored",
  code: "Code",
  long_context: "Long context",
  embedding: "Embedding",
};

const BADGE_STYLE: Partial<Record<Badge, string>> = {
  uncensored: "bg-ember/15 text-ember",
  vision: "bg-amber/15 text-amber",
  tools: "bg-ok/15 text-ok",
};

/** Headings between the groups in each section. */
const GROUP: Record<Fit, { label: string; style: string }> = {
  fits: { label: "Fits your PC", style: "text-ok" },
  partial: { label: "Bigger than your graphics card: runs, but slowly", style: "text-warn" },
  too_large: { label: "Too big for your machine", style: "text-danger" },
};

const FIT: Record<Fit, { label: string; style: string }> = {
  fits: { label: "Fits your GPU · fast", style: "bg-ok/15 text-ok" },
  partial: { label: "Runs, slower (uses system RAM)", style: "bg-warn/15 text-warn" },
  too_large: { label: "Too large for this PC", style: "bg-danger/15 text-danger" },
};

const SECTION: Record<Category, { title: string; blurb: string }> = {
  uncensored: {
    title: "Unfiltered",
    blurb: "Models with refusals removed (abliterated or fine-tuned). Qwen first, plus Gemma 4, GLM, gpt-oss, Granite and Dolphin.",
  },
  general: { title: "General", blurb: "Mainstream chat models, for comparison or when you want a filtered model." },
  coding: { title: "Coding", blurb: "For the coding agent later on." },
  embedding: { title: "Memory & search", blurb: "Not chat models. One of these powers memory and document search." },
};

const TIER_LABEL = {
  cpu: "CPU only",
  starter: "Starter tier",
  standard: "Standard tier",
  performance: "Performance tier",
  workstation: "Workstation tier",
} as const;

type Filter = "all" | "recommended" | "installed" | "vision" | "tools" | "uncensored";
const FILTERS: { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "recommended", label: "Best for this PC" },
  { id: "installed", label: "Installed" },
  { id: "uncensored", label: "Uncensored" },
  { id: "vision", label: "Vision" },
  { id: "tools", label: "Tools / Agent" },
];

const gb = (mb: number) => `${Math.round(mb / 1024)} GB`;
const bytes = (b: number) => (b >= 1073741824 ? `${(b / 1073741824).toFixed(1)} GB` : `${Math.round(b / 1048576)} MB`);

export default function Models({
  token,
  canManage,
  ollama,
  onEnginesChanged,
}: {
  token: string;
  canManage: boolean;
  ollama: EngineInfo | null;
  onEnginesChanged: () => void;
}) {
  const [data, setData] = useState<ModelsOverview | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setData(await api.modelsOverview(token));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Can't load models.");
    }
  }, [token]);

  const downloading = data?.pulls.some((p) => !p.done) ?? false;
  const ollamaState = ollama?.state;
  useEffect(() => {
    void refresh();
    const t = window.setInterval(() => void refresh(), downloading ? 1500 : 15000);
    return () => window.clearInterval(t);
  }, [refresh, downloading, ollamaState]);

  // Ollama may report a tag in other capitals (e.g. E4b-qat): compare names ignoring case.
  const installedSet = useMemo(() => new Set(data?.installed.map((m) => m.model.toLowerCase())), [data]);
  const installedNames = useMemo(() => ({ has: (m: string) => installedSet.has(m.toLowerCase()) }), [installedSet]);
  const pullFor = (model: string) => data?.pulls.find((p) => p.model === model);

  async function act(kind: "pull" | "remove", model: string) {
    setError(null);
    if (kind === "remove" && !window.confirm(`Remove ${model}? You can download it again later.`)) return;
    try {
      if (kind === "pull") {
        await api.pullModel(token, model);
        downloadsChanged();
      } else await api.removeModel(token, model);
      void refresh();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    }
  }

  const visible = (e: CatalogEntry) => {
    switch (filter) {
      case "all":
        return true;
      case "recommended":
        return e.recommended;
      case "installed":
        return installedNames.has(e.model);
      default:
        return e.badges.includes(filter);
    }
  };

  const catalogNames = new Set(data?.catalog.map((e) => e.model.toLowerCase()));
  const extraInstalled = data?.installed.filter((m) => !catalogNames.has(m.model.toLowerCase())) ?? [];
  const ollamaUp = data?.ollama_running ?? false;

  return (
    <section className="mx-auto flex max-w-5xl flex-col gap-6">
      <div className="flex flex-wrap items-end gap-4">
        <div className="flex-1">
          <h1 className="text-2xl font-semibold">Models</h1>
          <p className="text-muted">Picked for your hardware. Unfiltered Qwen models first.</p>
        </div>
        <Button variant="ghost" onClick={() => void refresh()}>
          Refresh
        </Button>
      </div>

      {data && <HardwareStrip hw={data.hardware} />}
      {!ollamaUp && ollama && (
        <EngineCard engine={ollama} token={token} canManage={canManage} onChanged={onEnginesChanged} />
      )}
      <ErrorNote>{error}</ErrorNote>

      <div className="flex flex-wrap gap-2" role="group" aria-label="Filter models">
        {FILTERS.map((f) => (
          <button
            key={f.id}
            type="button"
            aria-pressed={filter === f.id}
            onClick={() => setFilter(f.id)}
            className={`min-h-9 rounded-full border px-3.5 text-sm ${
              filter === f.id ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"
            }`}
          >
            {f.label}
          </button>
        ))}
      </div>

      {(["uncensored", "general", "coding", "embedding"] as Category[]).map((cat) => {
        const entries = data?.catalog.filter((e) => e.category === cat && visible(e)) ?? [];
        if (entries.length === 0) return null;
        return (
          <div key={cat} className="flex flex-col gap-3">
            <div>
              <h2 className="text-lg font-semibold">{SECTION[cat].title}</h2>
              <p className="text-sm text-muted">{SECTION[cat].blurb}</p>
            </div>
            <ul className="flex flex-col gap-3">
              {entries.map((e, i) => {
                // The gateway sorts: the pick, what fits, then what doesn't. Label each
                // change of group so it's clear where "fits this PC" ends.
                const prev = entries[i - 1];
                const heading = !e.recommended && (!prev || prev.recommended || prev.fit !== e.fit) ? GROUP[e.fit] : null;
                return (
                  <Fragment key={e.model}>
                    {heading && (
                      <li className={`mt-2 text-xs font-semibold tracking-wide uppercase ${heading.style}`}>{heading.label}</li>
                    )}
                    <CatalogRow
                      entry={e}
                      installed={installedNames.has(e.model)}
                      pull={pullFor(e.model)}
                      canManage={canManage && ollamaUp}
                      vramGb={(data?.hardware.vram_mb ?? 0) / 1024}
                      onPull={() => void act("pull", e.model)}
                      onRemove={() => void act("remove", e.model)}
                    />
                  </Fragment>
                );
              })}
            </ul>
          </div>
        );
      })}

      {extraInstalled.length > 0 && (filter === "all" || filter === "installed") && (
        <div className="flex flex-col gap-3">
          <h2 className="text-lg font-semibold">Also installed</h2>
          <ul className="overflow-hidden rounded-2xl border border-line">
            {extraInstalled.map((m) => (
              <InstalledRow key={m.model} m={m} canManage={canManage} onRemove={() => void act("remove", m.model)} />
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

function HardwareStrip({ hw }: { hw: Hardware }) {
  const gpu = hw.gpus[0];
  return (
    <div className="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-2xl border border-line bg-surface px-5 py-4 text-sm">
      <span className="rounded-full bg-ember px-3 py-1 text-xs font-semibold text-on-ember">{TIER_LABEL[hw.tier]}</span>
      <span>
        <span className="text-muted">GPU </span>
        {gpu ? `${gpu.name} · ${gb(gpu.vram_mb)}` : "none found (CPU only)"}
      </span>
      <span>
        <span className="text-muted">RAM </span>
        {gb(hw.ram_mb)}
      </span>
      <span>
        <span className="text-muted">Free disk </span>
        {gb(hw.disk_free_mb)}
      </span>
    </div>
  );
}

function Badges({ badges }: { badges: Badge[] }) {
  return (
    <span className="flex flex-wrap gap-1.5">
      {badges.map((b) => (
        <span key={b} className={`rounded-md px-2 py-0.5 text-xs font-medium ${BADGE_STYLE[b] ?? "bg-raised text-soft"}`}>
          {BADGE_LABEL[b]}
        </span>
      ))}
    </span>
  );
}

function CatalogRow({
  entry: e,
  installed,
  pull,
  canManage,
  vramGb,
  onPull,
  onRemove,
}: {
  entry: CatalogEntry;
  installed: boolean;
  pull: PullState | undefined;
  canManage: boolean;
  /** This machine's graphics card memory. */
  vramGb: number;
  onPull: () => void;
  onRemove: () => void;
}) {
  const pulling = pull && !pull.done;
  // How much of it won't fit on the card: a little spills and is a bit slower; a lot is far slower.
  const justOver = vramGb > 0 && e.size_gb * 1.15 + 0.75 <= vramGb * 1.15;
  const slower = justOver
    ? "a little of it runs in system memory, so replies are somewhat slower"
    : "a good part of it runs in system memory, so replies are several times slower";
  return (
    <li
      className={`flex flex-col gap-3 rounded-2xl border bg-surface p-4 ${
        e.recommended
          ? "border-pick"
          : e.fit === "too_large"
            ? "border-danger/40 opacity-75"
            : e.fit === "partial"
              ? "border-warn/40"
              : "border-line"
      }`}
    >
      <div className="flex flex-wrap items-start gap-3">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-semibold">{e.name}</span>
            {e.recommended && (
              <span className="rounded-full bg-pick px-2 py-0.5 text-xs font-semibold text-on-pick">Best for this PC</span>
            )}
          </div>
          <span className="font-mono text-xs text-muted">{e.model}</span>
          <p className="text-sm text-soft">{e.description}</p>
          {e.fit === "too_large" && !installed && (
            <p className="text-sm font-medium text-danger" role="note">
              This is too big for your machine. It needs about {Math.ceil(e.size_gb * 1.15 + 0.6)} GB of graphics and
              system memory to run.
            </p>
          )}
          {e.fit === "partial" && !installed && (
            <p className="text-sm text-warn" role="note">
              {justOver ? "Just bigger than your graphics card" : "Bigger than your graphics card"}: {slower}.
            </p>
          )}
        </div>
        <div className="flex flex-col items-end gap-2">
          {installed ? (
            <div className="flex items-center gap-2">
              <span className="text-sm text-ok">Installed</span>
              {canManage && (
                <Button variant="ghost" onClick={onRemove}>
                  Remove
                </Button>
              )}
            </div>
          ) : (
            canManage &&
            !pulling &&
            e.fit !== "too_large" && (
              <Button
                variant={e.fit === "partial" ? "ghost" : "primary"}
                onClick={() => {
                  if (
                    e.fit === "partial" &&
                    !window.confirm(
                      `${e.name} is ${justOver ? "just " : ""}bigger than your graphics card: ${slower}.\n\n` +
                        `Download ${e.size_gb} GB anyway?`,
                    )
                  )
                    return;
                  onPull();
                }}
              >
                Download · {e.size_gb} GB
              </Button>
            )
          )}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <span className={`rounded-md px-2 py-0.5 text-xs font-medium ${FIT[e.fit].style}`}>{FIT[e.fit].label}</span>
        <Badges badges={e.badges} />
        <span className="text-xs text-muted">
          ≈{e.size_gb} GB · {e.context_k}K context · {e.license ?? "licence: see model page"}
        </span>
        <a className="text-xs" href={e.page} target="_blank" rel="noreferrer">
          Model page
        </a>
      </div>
      {pulling && pull && (
        <div className="flex flex-col gap-1.5" aria-live="polite">
          <div className="flex justify-between text-xs text-muted">
            <span className="capitalize">{pull.status}</span>
            {pull.total > 0 && (
              <span>
                {bytes(pull.completed)} of {bytes(pull.total)}
                {pull.speed_bps ? ` · ${formatSpeed(pull.speed_bps)}` : ""}
                {pull.eta_secs != null ? ` · ${formatEta(pull.eta_secs)}` : ""}
              </span>
            )}
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-raised">
            {pull.percent !== null ? (
              <div className="h-2 rounded-full bg-ember transition-all" style={{ width: `${pull.percent}%` }} />
            ) : (
              <div className="h-2 w-1/3 animate-pulse rounded-full bg-ember/70" />
            )}
          </div>
        </div>
      )}
      {pull?.error && <ErrorNote>{pull.error}</ErrorNote>}
    </li>
  );
}

function InstalledRow({ m, canManage, onRemove }: { m: InstalledModel; canManage: boolean; onRemove: () => void }) {
  return (
    <li className="flex flex-wrap items-center gap-3 border-b border-line bg-surface px-5 py-3 last:border-b-0">
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="font-mono text-sm">{m.model}</span>
        <span className="flex flex-wrap items-center gap-2 text-xs text-muted">
          {[m.parameter_size, m.quantization, bytes(m.size_bytes)].filter(Boolean).join(" · ")}
          <Badges badges={m.badges} />
        </span>
      </span>
      {canManage && (
        <Button variant="ghost" onClick={onRemove}>
          Remove
        </Button>
      )}
    </li>
  );
}
