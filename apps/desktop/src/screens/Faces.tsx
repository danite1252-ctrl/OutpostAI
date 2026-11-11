// Faces (owners): the people Outpost can name at the door, and the faces seen there.

import { KnownFacesPanel } from "../components/faces";

export default function Faces({ token }: { token: string }) {
  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-6">
      <header>
        <h1 className="text-2xl font-semibold">Faces</h1>
        <p className="text-sm text-muted">Who Outpost can name at the door, and who it's seen there. Only owners see this.</p>
      </header>
      <KnownFacesPanel token={token} />
    </div>
  );
}
