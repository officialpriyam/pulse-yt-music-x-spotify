import { XMLParser } from "fast-xml-parser";
import { artwork } from "../../../catalog";
import type { PublicCatalog, YouTubeRelease } from "../../../youtube-catalog";

const sources = [
  { id: "UCq-Fj5jknLsUf-MWSy4_brA", artist: "T-Series", region: "india" },
  { id: "UC56gTxNs4f9xZ7Pa2i5xNzg", artist: "Sony Music India", region: "india" },
  { id: "UC0WP5P-ufpRfjbNrmOWwLBQ", artist: "The Weeknd", region: "global" },
  { id: "UC-J-KZfRV8c13fOCkhXdLiQ", artist: "Dua Lipa", region: "global" },
] as const;
const music = /(?:official (?:music )?video|(?:lyrics?|lyrical|audio|full video|visualizer)|live (?:at|from|in)|music video)/i;
const other = /(?:#shorts|playlist|teaser|trailer|behind the scenes)/i;
const maxFeedBytes = 256 * 1024;
const parser = new XMLParser({
  parseTagValue: false,
  isArray: (_tag, path) => path === "feed.entry",
});

type Feed = { feed?: { entry?: { "yt:videoId"?: unknown; title?: unknown; published?: unknown }[] } };

async function readFeed(source: typeof sources[number]): Promise<YouTubeRelease[]> {
  const url = new URL("https://www.youtube.com/feeds/videos.xml");
  url.searchParams.set("channel_id", source.id);
  const response = await fetch(url, {
    redirect: "error", signal: AbortSignal.timeout(5500), next: { revalidate: 1800 },
  });
  if (!response.ok || Number(response.headers.get("content-length")) > maxFeedBytes) return [];
  const xml = await response.text();
  // Atom feeds have no DTD. Decline entity definitions and bound parsing input.
  if (Buffer.byteLength(xml) > maxFeedBytes || /<!\s*(?:DOCTYPE|ENTITY)\b/i.test(xml)) return [];
  const feed = parser.parse(xml) as Feed;
  const entries = feed.feed?.entry;
  if (!Array.isArray(entries)) return [];
  return entries.flatMap((entry): YouTubeRelease[] => {
    const id = entry["yt:videoId"];
    const title = entry.title;
    const published = entry.published;
    if (typeof id !== "string" || !/^[A-Za-z0-9_-]{11}$/.test(id) ||
        typeof title !== "string" || title.length > 300 || !music.test(title) || other.test(title) ||
        typeof published !== "string" || !Number.isFinite(Date.parse(published))) return [];
    return [{ id, title, artist: source.artist, artwork: artwork(id), region: source.region, published }];
  }).slice(0, 7);
}

export async function GET() {
  const feeds = await Promise.allSettled(sources.map(readFeed));
  const tracks = feeds.flatMap((feed) => feed.status === "fulfilled" ? feed.value : [])
    .sort((a, b) => Date.parse(b.published) - Date.parse(a.published));
  const catalog: PublicCatalog = { tracks: [...new Map(tracks.map((track) => [track.id, track])).values()] };
  return Response.json(catalog, {
    headers: { "Cache-Control": tracks.length ? "public, s-maxage=900, stale-while-revalidate=3600" : "public, s-maxage=60" },
  });
}
