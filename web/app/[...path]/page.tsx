import { notFound } from "next/navigation";

const sections = new Set(["new", "radio", "search", "library", "settings", "login", "signup", "player"]);
const collections = new Set(["liked", "playlists", "youtube"]);

export default async function Page({ params }: { params: Promise<{ path: string[] }> }) {
  const { path } = await params;
  const [section, collection, id] = path;
  const valid =
    (path.length === 1 && sections.has(section)) ||
    (section === "radio" && path.length === 2 && ["night", "indie", "dance"].includes(collection)) ||
    (section === "library" && path.length === 2 && collections.has(collection)) ||
    (section === "library" && path.length === 3 && ["playlists", "youtube"].includes(collection) && /^[\w-]{1,150}$/.test(id));
  if (!valid) notFound();
  return null;
}
