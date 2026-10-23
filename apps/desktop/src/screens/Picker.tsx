import { useEffect, useState } from "react";
import { api, type DirectoryEntry } from "../api";
import { Avatar, ErrorNote, Mark, Wordmark } from "../components/ui";

const roleLabel = { owner: "Owner", member: "Member", restricted: "Restricted", service: "Service" } as const;

export default function Picker({
  machineName,
  onPick,
}: {
  machineName?: string;
  onPick: (account: DirectoryEntry) => void;
}) {
  const [accounts, setAccounts] = useState<DirectoryEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.directory().then(setAccounts, () => setError("Can't load accounts. Is the Outpost service running?"));
  }, []);

  return (
    <main className="flex h-full flex-col items-center justify-center gap-10 bg-deep px-6">
      <div className="flex items-center gap-3">
        <Mark size={44} />
        <Wordmark className="text-2xl" />
      </div>
      <h1 className="text-3xl font-semibold tracking-tight">Who's using Outpost?</h1>
      <ErrorNote>{error}</ErrorNote>
      <ul className="flex max-w-3xl flex-wrap justify-center gap-6">
        {accounts?.map((a) => (
          <li key={a.id}>
            <button
              type="button"
              onClick={() => onPick(a)}
              className="group flex w-32 flex-col items-center gap-3 rounded-2xl p-3 hover:bg-surface"
            >
              <span className="rounded-full ring-2 ring-transparent ring-offset-4 ring-offset-deep group-hover:ring-ember">
                <Avatar name={a.name} color={a.avatar_color} size={88} />
              </span>
              <span className="font-medium">{a.name}</span>
              <span className="text-xs text-muted">{roleLabel[a.role]}</span>
            </button>
          </li>
        ))}
      </ul>
      {machineName && <p className="text-xs text-muted">Running on {machineName}</p>}
    </main>
  );
}
