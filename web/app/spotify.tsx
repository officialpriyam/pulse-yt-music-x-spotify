"use client";

import Link from "next/link";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { Headphones, Pause, Play, Search, SkipBack, SkipForward, Volume2 } from "lucide-react";
import { api, describe, type Session } from "./catalog";

type SpotifyTrack = {
  id: string;
  name: string;
  uri: string;
  artists: { name: string }[];
  album: { images: { url: string }[] };
  external_urls: { spotify: string };
};
type Playback = {
  position: number;
  duration: number;
  paused: boolean;
  track_window: { current_track: SpotifyTrack };
};
type Player = {
  addListener: (event: string, listener: (payload: never) => void) => boolean;
  connect: () => Promise<boolean>;
  disconnect: () => void;
  activateElement: () => Promise<void>;
  togglePlay: () => Promise<void>;
  previousTrack: () => Promise<void>;
  nextTrack: () => Promise<void>;
  seek: (position: number) => Promise<void>;
  setVolume: (volume: number) => Promise<void>;
  getCurrentState: () => Promise<Playback | null>;
};
type SpotifySDK = { Player: new (config: { name: string; getOAuthToken: (callback: (token: string) => void) => void }) => Player };
type SdkWindow = Window & { Spotify?: SpotifySDK; onSpotifyWebPlaybackSDKReady?: () => void };
let loading: Promise<SpotifySDK> | undefined;

function loadSdk(): Promise<SpotifySDK> {
  const browser = window as SdkWindow;
  if (browser.Spotify) return Promise.resolve(browser.Spotify);
  if (!loading) loading = new Promise<SpotifySDK>((resolve, reject) => {
    const script = document.createElement("script");
    const fail = () => { script.remove(); delete browser.onSpotifyWebPlaybackSDKReady; reject(new Error("Could not load the Spotify player.")); };
    const timer = window.setTimeout(fail, 15_000);
    browser.onSpotifyWebPlaybackSDKReady = () => {
      window.clearTimeout(timer);
      delete browser.onSpotifyWebPlaybackSDKReady;
      if (browser.Spotify) resolve(browser.Spotify);
      else fail();
    };
    script.src = "https://sdk.scdn.co/spotify-player.js";
    script.async = true;
    script.onerror = () => { window.clearTimeout(timer); fail(); };
    document.head.append(script);
  }).catch((error) => { loading = undefined; throw error; });
  return loading;
}

async function spotify<T>(path: string, init?: RequestInit) {
  const { access_token } = await api<{ access_token: string }>("spotify/token", { method: "POST" });
  const response = await fetch(`https://api.spotify.com/v1/${path}`, {
    ...init,
    headers: { Authorization: `Bearer ${access_token}`, ...(init?.body ? { "Content-Type": "application/json" } : {}) },
    cache: "no-store",
  });
  if (!response.ok) {
    if (response.status === 403) throw new Error("Spotify denied access. Check Premium and your app access in Spotify.");
    if (response.status === 429) throw new Error("Spotify is rate limiting this request. Try again shortly.");
    throw new Error("Spotify could not complete the request.");
  }
  return response.status === 204 || response.headers.get("content-length") === "0" ? undefined as T : response.json() as Promise<T>;
}

function time(ms: number) {
  const seconds = Math.floor(ms / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export default function SpotifyPage({ account }: { account?: Session }) {
  const [requested, request] = useState(0);
  const [device, setDevice] = useState("");
  const [state, setPlayback] = useState<Playback>();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SpotifyTrack[]>([]);
  const [searched, setSearched] = useState("");
  const [more, setMore] = useState(false);
  const [searching, setSearching] = useState(false);
  const [command, setCommand] = useState(false);
  const [volume, setVolume] = useState(100);
  const [error, setError] = useState("");
  const player = useRef<Player>(null);

  useEffect(() => {
    if (!requested || !account?.spotify_connected) return;
    let active = true;
    let sdk: Player | undefined;
    setDevice("");
    setPlayback(undefined);
    setError("");
    async function setup() {
      if (!window.isSecureContext || !navigator.requestMediaKeySystemAccess) {
        throw new Error("This browser needs a secure connection and protected-media support for Spotify.");
      }
      const { Player } = await loadSdk();
      if (!active) return;
      sdk = new Player({ name: "Pulse Web", getOAuthToken: (send) => {
        api<{ access_token: string }>("spotify/token", { method: "POST" }).then(({ access_token }) => {
          if (active) send(access_token);
        }).catch((error) => { if (active) setError(describe(error)); });
      } });
      sdk.addListener("ready", ({ device_id }: { device_id: string }) => { if (active) setDevice(device_id); });
      sdk.addListener("not_ready", () => { if (active) setDevice(""); });
      sdk.addListener("player_state_changed", (value: Playback | null) => { if (active) setPlayback(value ?? undefined); });
      sdk.addListener("account_error", () => { if (active) setError("Spotify Premium is required for web playback."); });
      sdk.addListener("authentication_error", () => { if (active) setError("Reconnect Spotify in Settings."); });
      sdk.addListener("initialization_error", () => { if (active) setError("Spotify could not initialize protected playback."); });
      sdk.addListener("playback_error", () => { if (active) setError("Spotify could not play this track."); });
      player.current = sdk;
      if (!(await sdk.connect())) throw new Error("Could not connect to Spotify.");
    }
    void setup().catch((error) => { if (active) setError(describe(error)); });
    const timer = window.setInterval(() => {
      void sdk?.getCurrentState().then((value) => { if (active && value) setPlayback(value); }).catch(() => {});
    }, 1000);
    return () => {
      active = false;
      window.clearInterval(timer);
      sdk?.disconnect();
      player.current = null;
    };
  }, [requested, account?.spotify_connected]);

  async function find(event?: FormEvent, paginate = false) {
    event?.preventDefault();
    const q = (paginate ? searched : query).trim();
    if (!q || searching) return;
    setSearching(true);
    setError("");
    if (!paginate) { setResults([]); setMore(false); }
    try {
      const offset = paginate ? results.length : 0;
      const params = new URLSearchParams({ q, type: "track", limit: "10", offset: String(offset) });
      const data = await spotify<{ tracks: { items: SpotifyTrack[]; next: string | null } }>(`search?${params}`);
      setResults((before) => paginate ? [...before, ...data.tracks.items] : data.tracks.items);
      setMore(!!data.tracks.next);
      setSearched(q);
    } catch (error) { setError(describe(error)); }
    finally { setSearching(false); }
  }

  async function control(task: (instance: Player) => Promise<unknown>) {
    if (!device || !player.current || command) return;
    setCommand(true);
    setError("");
    try { await task(player.current); }
    catch (error) { setError(describe(error)); }
    finally { setCommand(false); }
  }

  return <section className="screen">
    <h1>Spotify player</h1>
    {!account?.spotify_connected ? <div className="empty-state glass"><Headphones size={27} />
      <h2>Listen in your browser</h2><p>Connect a Spotify Premium account to play audio here.</p>
      <Link className="primary" href={account?.signed_in ? "/settings" : "/login"}>{account?.signed_in ? "Open Connections" : "Sign in to Pulse"}</Link>
    </div> : <>
      <div className="settings-card spotify-player glass">
        {state?.track_window.current_track.album.images[0] && <img src={state.track_window.current_track.album.images[0].url} alt="" />}
        <div>
          <h2>{state?.track_window.current_track.name ?? "Pulse Web"}</h2>
          <p>{state?.track_window.current_track.artists.map((artist) => artist.name).join(", ") ?? (device ? "Choose a Spotify track below." : "Enable the player to start playback.")}</p>
          {!device && <button className="quiet-button" onClick={() => request((value) => value + 1)}>
            <Headphones size={17} /> {requested ? "Reconnect player" : "Enable player"}
          </button>}
        </div>
        {state && device && <div className="spotify-controls">
          <div>
            <button className="icon-button" aria-label="Previous track" disabled={command} onClick={() => control((instance) => instance.previousTrack())}><SkipBack size={19} /></button>
            <button className="primary" aria-label={state.paused ? "Play" : "Pause"} disabled={command} onClick={() => control((instance) => instance.togglePlay())}>{state.paused ? <Play size={18} /> : <Pause size={18} />}</button>
            <button className="icon-button" aria-label="Next track" disabled={command} onClick={() => control((instance) => instance.nextTrack())}><SkipForward size={19} /></button>
          </div>
          <div><small>{time(state.position)}</small><input type="range" aria-label="Playback position" min={0} max={state.duration}
            value={state.position} onChange={(event) => setPlayback({ ...state, position: Number(event.target.value) })}
            onPointerUp={(event) => { const position = Number(event.currentTarget.value); void control((instance) => instance.seek(position)); }}
            onKeyUp={(event) => { const position = Number(event.currentTarget.value); void control((instance) => instance.seek(position)); }} /><small>{time(state.duration)}</small></div>
          <div><Volume2 size={15} /><input type="range" aria-label="Volume" min={0} max={100} value={volume} onChange={(event) => {
            const value = Number(event.target.value); setVolume(value);
            void player.current?.setVolume(value / 100).catch(() => setError("Volume could not be changed."));
          }} /></div>
        </div>}
      </div>
      {error && <p role="alert" className="form-error">{error}</p>}
      <form className="search-form glass section" onSubmit={find}>
        <Search size={20} /><input aria-label="Search Spotify" placeholder="Search Spotify for a track" value={query} maxLength={120} onChange={(event) => setQuery(event.target.value)} />
        <button className="primary" disabled={searching || !query.trim()}>Search</button>
      </form>
      {searched && !searching && !results.length && <p className="hint">No tracks found for “{searched}”.</p>}
      <ul className="track-list section">{results.map((track) => <li className="track-row" key={track.id}>
        <button className="track-open" disabled={!device || command} onClick={() => control(async (instance) => {
          await instance.activateElement();
          await spotify(`me/player/play?device_id=${encodeURIComponent(device)}`, { method: "PUT", body: JSON.stringify({ uris: [track.uri] }) });
        })}>
          <span className="cover">{track.album.images[0] && <img src={track.album.images[0].url} alt="" />}</span>
          <span className="track-text"><strong>{track.name}</strong><small>{track.artists.map((artist) => artist.name).join(", ")}</small></span>
        </button>
        <a className="quiet-button" href={track.external_urls.spotify} target="_blank" rel="noreferrer">Spotify</a>
      </li>)}</ul>
      {more && <button className="quiet-button" disabled={searching} onClick={() => find(undefined, true)}>Load more</button>}
    </>}
  </section>;
}
