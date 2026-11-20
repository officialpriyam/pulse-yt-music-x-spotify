import { artwork, type Track } from "./catalog";

export const youtubeHome: Track[] = [
  ["BddP6PYo2gs", "Kesariya", "Arijit Singh, Pritam"],
  ["dX3k_QDnzHE", "Midnight City", "M83"],
  ["4NRXx6U8ABQ", "Blinding Lights", "The Weeknd"],
  ["sBzrzS1Ag_g", "The Less I Know the Better", "Tame Impala"],
  ["5NV6Rdv1a3I", "Get Lucky", "Daft Punk"],
  ["bpOSxM0rNPM", "Do I Wanna Know?", "Arctic Monkeys"],
  ["TUVcZfQe-Kw", "Levitating", "Dua Lipa"],
].map(([id, title, artist]) => ({ id, title, artist, artwork: artwork(id) }));

export const youtubeMixes = [
  { id: "india", title: "Bollywood", description: "Explore Hindi film music and current releases." },
  { id: "global", title: "Global pop", description: "Explore pop, dance and alternative music." },
] as const;

export type YouTubeRelease = Track & { region: "india" | "global"; published: string };
export type PublicCatalog = { tracks: YouTubeRelease[] };

export function youtubeTrackUrl(id: string) {
  return `https://music.youtube.com/watch?v=${encodeURIComponent(id)}`;
}

export function youtubeSearchUrl(query: string) {
  return `https://music.youtube.com/search?q=${encodeURIComponent(query)}`;
}
