export type Track = { id: string; title: string; artist: string; artwork: string; audio?: string; source?: string };
export type Playlist = { id: string; title: string; artwork: string; count: number };

type Item = {
  id: string | { videoId?: string };
  snippet?: {
    title?: string;
    channelTitle?: string;
    videoOwnerChannelTitle?: string;
    resourceId?: { videoId?: string };
    thumbnails?: Record<string, { url: string }>;
  };
  contentDetails?: { videoId?: string; itemCount?: number };
};

export type YoutubePage = { items?: Item[]; ids?: string[]; nextPageToken?: string };

export function artwork(id: string) {
  return `https://i.ytimg.com/vi/${id}/mqdefault.jpg`;
}

const openTracks = [
  ["space-jazz", "Space Jazz"],
  ["night-on-docks", "Night on the Docks"],
  ["local-forecast", "Local Forecast"],
  ["disco-medusae", "Disco Medusae"],
  ["c-funk", "C-Funk"],
] as const;

export const featured: Track[] = openTracks.map(([slug, title]) => ({
  id: `pulse-${slug}`,
  title,
  artist: "Kevin MacLeod",
  artwork: "",
  audio: `/audio/${slug}.mp3`,
  source: `https://incompetech.com/music/royalty-free/index.html?keywords=${encodeURIComponent(title)}`,
}));
export const audioLicense = "https://creativecommons.org/licenses/by/4.0/";

function thumbnail(item: Item) {
  const images = item.snippet?.thumbnails;
  const url = images?.maxres?.url || images?.high?.url || images?.medium?.url;
  if (url?.startsWith("https://")) return url;
  return "";
}

export function tracks(page: YoutubePage): Track[] {
  return (page.items || []).flatMap((item) => {
    const id = item.contentDetails?.videoId || item.snippet?.resourceId?.videoId ||
      (typeof item.id === "string" ? item.id : item.id.videoId);
    const title = item.snippet?.title;
    const builtIn = featured.find((item) => item.id === id);
    if (builtIn) return [builtIn];
    if (!id || !/^[\w-]{11}$/.test(id) || !title || ["Private video", "Deleted video"].includes(title)) return [];
    return [{
      id, title, artist: item.snippet?.videoOwnerChannelTitle || item.snippet?.channelTitle || "YouTube",
      artwork: thumbnail(item) || artwork(id),
    }];
  });
}

export function playlists(page: YoutubePage): Playlist[] {
  return (page.items || []).flatMap((item) => {
    if (typeof item.id !== "string" || !item.snippet?.title) return [];
    return [{ id: item.id, title: item.snippet.title, artwork: thumbnail(item), count: item.contentDetails?.itemCount || 0 }];
  });
}

export async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path.startsWith("/auth/") ? path : `/api/v1/${path}`, {
    ...init,
    headers: { "X-Pulse-Request": "1", ...(init?.body ? { "Content-Type": "application/json" } : {}) },
    cache: "no-store",
  });
  if (!response.ok) {
    const message = await response.text();
    throw new Error(message && !message.startsWith("<") && message.length < 180 ? message : "Pulse API is unavailable. Try again.");
  }
  const raw = await response.text();
  return raw ? JSON.parse(raw) as T : undefined as T;
}

export type Session = {
  configured: boolean;
  signed_in: boolean;
  name?: string;
  username?: string | null;
  google_available: boolean;
  discord_available: boolean;
  spotify_available: boolean;
  google_connected: boolean;
  discord_linked: boolean;
  youtube_connected: boolean;
  spotify_connected: boolean;
};

export function describe(error: unknown) {
  return error instanceof Error ? error.message : "Something went wrong. Try again.";
}

export const stations = [
  { id: "night", title: "After Hours", description: "Space jazz and mellow instrumentals.", tracks: [0, 1, 2] },
  { id: "indie", title: "Jazz Lounge", description: "Mellow jazz for a slow afternoon.", tracks: [2, 1, 0] },
  { id: "dance", title: "Dance Floor", description: "Disco grooves and funk.", tracks: [3, 4] },
] as const;
