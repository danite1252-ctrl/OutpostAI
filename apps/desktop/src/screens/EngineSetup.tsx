import { useState } from "react";
import { api, type SignInResponse } from "../api";
import { EnginesPanel, useEngines } from "../components/engines";
import { Button, Mark, Wordmark } from "../components/ui";

/** First-run step after the owner account: install Ollama and ComfyUI. */
export default function EngineSetup({ auth, onDone }: { auth: SignInResponse; onDone: () => void }) {
  const { status, refresh } = useEngines(auth.token);
  const [startedAll, setStartedAll] = useState(false);

  const toInstall = status?.engines.filter((e) => e.state === "not_installed") ?? [];
  const anyBusy = status?.engines.some((e) => e.state === "installing") ?? false;

  async function installAll() {
    setStartedAll(true);
    for (const e of toInstall) {
      await api.engineAction(auth.token, e.id, "install").catch(() => undefined);
    }
    void refresh();
  }

  return (
    <main className="h-full overflow-auto bg-deep">
      <div className="mx-auto flex max-w-4xl flex-col gap-8 px-8 py-12">
        <div className="flex items-center gap-3">
          <Mark size={40} />
          <Wordmark className="text-xl" />
        </div>
        <div className="flex flex-col gap-2">
          <h1 className="text-3xl font-semibold tracking-tight">Set up your engines</h1>
          <p className="max-w-2xl text-muted">
            Outpost installs these into its own folder and runs them for you. Downloads come from the official Ollama,
            uv, Python, PyTorch and ComfyUI releases; after that everything runs on this PC.
          </p>
        </div>

        <EnginesPanel token={auth.token} canManage status={status} onChanged={() => void refresh()} />

        <div className="flex flex-wrap items-center gap-3">
          {toInstall.length > 0 && !startedAll && (
            <Button onClick={() => void installAll()}>Install {toInstall.length === 2 ? "both" : "it"}</Button>
          )}
          <Button variant={toInstall.length > 0 && !startedAll ? "ghost" : "primary"} onClick={onDone}>
            {anyBusy ? "Continue, keep installing in the background" : "Continue to Outpost"}
          </Button>
        </div>
        <p className="text-xs text-muted">You can install, stop or reinstall engines later in Settings.</p>
      </div>
    </main>
  );
}
