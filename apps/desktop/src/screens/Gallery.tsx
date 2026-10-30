import { useCallback, useEffect, useRef, useState } from "react";
import { api, ApiError, createApi, type Account, type Media, type Pack, type SearchStatus, type SharedMedia } from "../api";
import { GalleryGrid, Viewer, type CreateRequest } from "../components/gallery";
import { dropCached } from "../components/images";
import { Avatar, Button, ErrorNote } from "../components/ui";

type Filter = "all" | "image" | "video";
/** My gallery, the shared one, or (owner only) another account's. */
type Tab = { kind: "mine" } | { kind: "shared" } | { kind: "account"; account: Account };

type Item = Media & Partial<Pick<SharedMedia, "by_name" | "by_color">>;

const tabKey = (t: Tab) => (t.kind === "account" ? `account:${t.account.id}` : t.kind);

export default function Gallery({
  token,
  me,
  onRequest,
  onUnauthorized,
}: {
  token: string;
  me: Account;
  onRequest: (r: CreateRequest) => void;
  onUnauthorized: () => void;
}) {
  const isOwner = me.role === "owner";
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [tab, setTab] = useState<Tab>({ kind: "mine" });
  const [items, setItems] = useState<Item[]>([]);
  const [packs, setPacks] = useState<Pack[]>([]);
  const [more, setMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<Filter>("all");
  const [viewing, setViewing] = useState<Item | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Searching my gallery: the words, the results (null while not searching), and how far
  // describing pictures has got.
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Item[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [status, setStatus] = useState<SearchStatus | null>(null);
  // The tab showing now, so a slow answer for another tab is dropped.
  const showing = useRef(tabKey(tab));
  showing.current = tabKey(tab);

  const fail = useCallback(
    (err: unknown) => {
      if (err instanceof ApiError && err.status === 401) return onUnauthorized();
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    },
    [onUnauthorized],
  );

  const key = tabKey(tab);
  const load = useCallback(
    async (before?: number, have: Item[] = []) => {
      setLoading(true);
      setError(null);
      try {
        const page: Item[] =
          tab.kind === "shared"
            ? await createApi.sharedGallery(token, before)
            : await createApi.gallery(token, before, tab.kind === "account" ? tab.account.id : undefined);
        if (showing.current !== key) return;
        // Pages overlap by a second; keep each item once.
        const seen = new Set(have.map((m) => m.id));
        const fresh = page.filter((m) => !seen.has(m.id));
        setItems([...have, ...fresh]);
        setMore(page.length >= 60 && fresh.length > 0);
      } catch (err) {
        fail(err);
      } finally {
        setLoading(false);
      }
    },
    [token, fail, key], // eslint-disable-line react-hooks/exhaustive-deps
  );

  useEffect(() => {
    setItems([]);
    setMore(false);
    void load();
  }, [load]);

  useEffect(() => {
    void createApi.packs(token).then(setPacks, () => undefined);
    if (isOwner) void api.listAccounts(token).then(setAccounts, () => undefined);
  }, [token, isOwner]);

  useEffect(() => {
    void createApi.searchStatus(token).then(setStatus, () => undefined);
  }, [token]);
  // Search a moment after typing stops.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults(null);
      return;
    }
    const t = window.setTimeout(() => {
      setSearching(true);
      createApi
        .searchGallery(token, q)
        .then(setResults, fail)
        .finally(() => setSearching(false));
    }, 350);
    return () => window.clearTimeout(t);
  }, [query, token, fail]);

  const mineItem = (m: Item) => m.account_id === me.id;

  function nameOf(m: Item) {
    return m.by_name ?? accounts.find((a) => a.id === m.account_id)?.name ?? "someone";
  }

  async function remove(m: Item) {
    const whose = mineItem(m) ? "your gallery" : `${nameOf(m)}'s gallery`;
    const also = m.shared_at ? " It goes from the shared gallery too." : "";
    if (!window.confirm(`Delete this from ${whose}?${also}`)) return;
    try {
      await createApi.deleteMedia(token, m.id);
      dropCached(`/api/media/${m.id}`);
      setItems((g) => g.filter((x) => x.id !== m.id));
      setViewing(null);
    } catch (err) {
      fail(err);
    }
  }

  async function share(m: Item, shared: boolean) {
    try {
      const updated = await createApi.shareMedia(token, m.id, shared);
      const merged: Item = { ...m, shared_at: updated.shared_at };
      if (tab.kind === "shared" && !shared) {
        setItems((list) => list.filter((x) => x.id !== m.id));
        setViewing(null);
      } else {
        setItems((list) => list.map((x) => (x.id === m.id ? merged : x)));
        setViewing(merged);
      }
    } catch (err) {
      fail(err);
    }
  }

  const searchingMine = tab.kind === "mine" && results !== null;
  const shown = (searchingMine ? results : items).filter((m) => filter === "all" || m.kind === filter);
  const last = items[items.length - 1];
  const oldest = last ? (tab.kind === "shared" ? (last.shared_at ?? undefined) : last.created_at) : undefined;
  const others = accounts.filter((a) => a.id !== me.id);

  const intro =
    tab.kind === "mine"
      ? "Everything you've made in Create. Private unless you share it."
      : tab.kind === "shared"
        ? "Pictures and videos people here chose to share with everyone. Open one of yours and press Share with everyone to add it."
        : `Everything ${tab.account.name} made in Create. Only owners can see this.`;
  const empty =
    tab.kind === "mine"
      ? "Nothing yet. Pictures and videos you make in Create appear here."
      : tab.kind === "shared"
        ? "Nothing shared yet."
        : `${tab.account.name} hasn't made anything yet.`;

  const tabButton = (t: Tab, label: string, avatar?: Account) => (
    <button
      key={tabKey(t)}
      type="button"
      role="tab"
      aria-selected={tabKey(t) === key}
      onClick={() => setTab(t)}
      className={`flex min-h-11 items-center gap-2 rounded-xl border px-3 ${
        tabKey(t) === key ? "border-ember bg-raised" : "border-line bg-surface hover:bg-raised"
      }`}
    >
      {avatar && <Avatar name={avatar.name} color={avatar.avatar_color} size={24} />}
      <span className="text-sm font-medium">{label}</span>
    </button>
  );

  return (
    <section className="mx-auto flex max-w-6xl flex-col gap-6">
      <div className="flex flex-wrap items-end gap-4">
        <div className="flex-1">
          <h1 className="text-2xl font-semibold">Gallery</h1>
          <p className="text-sm text-muted">{intro}</p>
        </div>
        <div className="flex gap-2" role="radiogroup" aria-label="Show">
          {(
            [
              ["all", "All"],
              ["image", "Pictures"],
              ["video", "Videos"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="radio"
              aria-checked={filter === id}
              onClick={() => setFilter(id)}
              className={`min-h-10 rounded-lg border px-3 text-sm ${
                filter === id ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2" role="tablist" aria-label="Which gallery">
        {tabButton({ kind: "mine" }, "Mine", me)}
        {tabButton({ kind: "shared" }, "Shared")}
        {isOwner && others.length > 0 && (
          <>
            <span className="mx-1 h-6 w-px bg-line" aria-hidden="true" />
            <span className="text-xs text-muted">Everyone's (only you see these):</span>
            {others.map((a) => tabButton({ kind: "account", account: a }, a.name, a))}
          </>
        )}
      </div>
      {tab.kind === "mine" && (
        <div className="flex flex-col gap-1.5">
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search by what's in them, e.g. dog on a beach"
            aria-label="Search my gallery"
            className="min-h-11 rounded-xl border border-line bg-surface px-3.5 text-base text-ink placeholder:text-muted/60 focus:border-ember focus:outline-none"
          />
          {status && <SearchNote status={status} isOwner={isOwner} onToggle={(on) => void createApi.setSearch(token, on).then(setStatus, fail)} />}
        </div>
      )}
      <ErrorNote>{error}</ErrorNote>

      {searchingMine && !searching && shown.length === 0 ? (
        <p className="text-sm text-muted">Nothing matches “{query.trim()}”.</p>
      ) : !searchingMine && !loading && shown.length === 0 ? (
        <p className="text-sm text-muted">{items.length === 0 ? empty : "Nothing of that kind yet."}</p>
      ) : (
        <GalleryGrid
          token={token}
          items={shown}
          onOpen={setViewing}
          caption={tab.kind === "shared" ? (m) => (mineItem(m) ? "Shared by you" : `Shared by ${nameOf(m)}`) : undefined}
        />
      )}
      {more && !searchingMine && (
        <div className="flex justify-center">
          <Button variant="ghost" disabled={loading} onClick={() => void load(oldest, items)}>
            {loading ? "Loading…" : "Show older"}
          </Button>
        </div>
      )}

      {viewing && (
        <Viewer
          token={token}
          media={viewing}
          packName={packs.find((p) => p.id === viewing.pack_id)?.name}
          by={mineItem(viewing) ? undefined : nameOf(viewing)}
          onClose={() => setViewing(null)}
          onRequest={(r) => {
            setViewing(null);
            onRequest(r);
          }}
          // Share your own; an owner can also take anyone's out of the shared gallery.
          onShare={mineItem(viewing) || (isOwner && viewing.shared_at) ? (shared) => void share(viewing, shared) : undefined}
          onDelete={mineItem(viewing) || isOwner ? () => void remove(viewing) : undefined}
        />
      )}
    </section>
  );
}

/** Under the search box: how much of the gallery search can see into. */
function SearchNote({ status, isOwner, onToggle }: { status: SearchStatus; isOwner: boolean; onToggle: (on: boolean) => void }) {
  let text: string;
  if (!status.describer) {
    text = "Searching prompts only. Get a model that can see pictures (the eye badge in Models) and Outpost describes each picture so you can search by what's in it.";
  } else if (!status.enabled) {
    text = "Searching prompts and the pictures described so far; describing is turned off.";
  } else if (status.waiting > 0) {
    text = `Outpost has described ${status.described} of ${status.total} pictures so far (it works while the graphics card is free); the rest are found by their prompts.`;
  } else {
    text = "Searching what's in every picture, and its prompt.";
  }
  return (
    <p className="text-xs text-muted">
      {text}
      {!status.embedder && " Install an embedding model (Qwen3 Embedding 0.6B in Models) to find pictures by meaning, not just matching words."}
      {isOwner && status.describer && (
        <>
          {" "}
          <button type="button" className="underline hover:text-ink" onClick={() => onToggle(!status.enabled)}>
            {status.enabled ? "Stop describing" : "Start describing"}
          </button>
        </>
      )}
    </p>
  );
}
