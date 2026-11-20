"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import {
  ArrowLeft, ArrowRight, AudioLines, CircleAlert, Disc3, Headphones, Heart, House,
  LibraryBig, ListMusic, LogIn, Music2, Plus, Radio, Search, Settings2, Sparkles, Trash2, X, MonitorPlay,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { AccountPage, Settings } from "./account";
import { api, describe, featured, playlists, stations, tracks, type Playlist, type Session, type Track, type YoutubePage } from "./catalog";
import SpotifyPage from "./spotify";

const navigation = [
  { href: "/", title: "Home", icon: House },
  { href: "/new", title: "New", icon: Sparkles },
  { href: "/radio", title: "Radio", icon: Radio },
  { href: "/library", title: "Library", icon: LibraryBig },
  { href: "/search", title: "Search", icon: Search },
] as const;

type PulsePlaylist = Omit<Playlist, "artwork">;
type Collection = { items: PulsePlaylist[] };
type PlaylistDetail = YoutubePage & { id: string; title: string };

function Cover({ src, className = "" }: { src?: string; className?: string }) {
  const [failed, setFailed] = useState("");
  return <span className={`cover ${className}`}>
    {src && failed !== src ? <img src={src} alt="" loading="lazy" onError={() => setFailed(src)} /> : <Music2 aria-hidden />}
  </span>;
}

function useResource<T>(path: string | undefined, revision = 0) {
  const [result, setResult] = useState<{ path: string; revision: number; data?: T; error?: string }>();
  useEffect(() => {
    if (!path) return;
    const controller = new AbortController();
    api<T>(path, { signal: controller.signal }).then((data) => {
      if (!controller.signal.aborted) setResult({ path, revision, data });
    }).catch((error) => {
      if (!controller.signal.aborted) setResult({ path, revision, error: describe(error) });
    });
    return () => controller.abort();
  }, [path, revision]);
  const current = path && result?.path === path && result.revision === revision ? result : undefined;
  return { data: current?.data, error: current?.error, loading: !!path && !current };
}

function Feedback({ loading, error, retry }: { loading?: boolean; error?: string; retry?: () => void }) {
  if (loading) return <p className="hint" role="status">Loading…</p>;
  if (!error) return null;
  return <div className="inline-error" role="alert"><CircleAlert size={17} /><span>{error}</span>
    {retry && <button className="quiet-button" onClick={retry}>Retry</button>}</div>;
}

function usePages<T>(enabled: boolean, path: string, parse: (page: YoutubePage) => T[]) {
  const [result, setResult] = useState<{ path: string; items: T[]; next?: string; error?: string }>();
  const [paging, setPaging] = useState(false);
  const [pageError, setPageError] = useState("");
  const [retry, setRetry] = useState(0);
  const request = useRef<AbortController | null>(null);
  useEffect(() => {
    if (!enabled || !path) return;
    const controller = new AbortController();
    api<YoutubePage>(path, { signal: controller.signal }).then((page) => {
      if (!controller.signal.aborted) setResult({ path, items: parse(page), next: page.nextPageToken });
    }).catch((error) => {
      if (!controller.signal.aborted) setResult({ path, items: [], error: describe(error) });
    });
    return () => { controller.abort(); request.current?.abort(); };
  }, [enabled, parse, path, retry]);
  const current = enabled && result?.path === path ? result : undefined;
  async function next() {
    if (!current?.next || paging) return;
    const controller = new AbortController();
    request.current = controller;
    setPaging(true);
    setPageError("");
    try {
      const params = new URLSearchParams({ page_token: current.next });
      const page = await api<YoutubePage>(`${path}${path.includes("?") ? "&" : "?"}${params}`, { signal: controller.signal });
      if (!controller.signal.aborted) {
        setResult({ path, items: [...current.items, ...parse(page)], next: page.nextPageToken });
      }
    } catch (error) { if (!controller.signal.aborted) setPageError(describe(error)); }
    finally { if (request.current === controller) { setPaging(false); request.current = null; } }
  }
  return {
    items: current?.items ?? [],
    loading: !!path && enabled && !current,
    error: current?.error,
    pageError, paging, next, hasNext: !!current?.next,
    retry: () => { setResult(undefined); setRetry((value) => value + 1); },
  };
}

type MusicRows = (items: Track[], remove?: (item: Track) => void) => ReactNode;

function SearchPage({ connected, rows }: { connected: boolean; rows: MusicRows }) {
  const [query, setQuery] = useState("");
  const [submitted, setSubmitted] = useState("");
  const path = submitted ? `youtube/search?${new URLSearchParams({ q: submitted })}` : "";
  const remote = usePages(connected, path, tracks);
  const local = submitted ? featured.filter((track) => `${track.title} ${track.artist}`.toLowerCase().includes(submitted.toLowerCase())) : [];
  const results = connected ? remote.items : local;
  function search(event: FormEvent) { event.preventDefault(); setSubmitted(query.trim()); }
  return <section className="screen">
    <h1>Search</h1>
    <form className="search-form glass" onSubmit={search}>
      <Search size={20} /><input aria-label="Search tracks and artists" value={query} maxLength={120}
        placeholder="Search tracks and artists" onChange={(event) => setQuery(event.target.value)} />
      <button className="primary" type="submit" disabled={!query.trim()}>Search</button>
    </form>
    {!connected && <p className="hint">Search featured tracks or <Link href="/settings">connect YouTube</Link> to search its catalog.</p>}
    {submitted && <h2 className="results-heading">Results for “{submitted}”</h2>}
    <Feedback loading={remote.loading} error={remote.error} retry={remote.retry} />
    {results.length > 0 && rows(results)}
    {submitted && !remote.loading && !remote.error && !results.length && <p className="hint">No tracks found.</p>}
    {remote.pageError && <Feedback error={remote.pageError} />}
    {remote.hasNext && <button className="quiet-button" disabled={remote.paging} onClick={remote.next}>Load more</button>}
  </section>;
}

function NewMusic({ connected, rows }: { connected: boolean; rows: MusicRows }) {
  const [revision, refresh] = useState(0);
  const popular = useResource<YoutubePage>(connected ? "youtube/popular" : undefined, revision);
  const songs = popular.data ? tracks(popular.data) : featured;
  return <section className="screen">
    <h1>New</h1>
    <div className="section-head"><h2>{popular.data ? "Popular on YouTube" : "Featured music"}</h2>
      {connected && <button className="quiet-button" onClick={() => refresh((value) => value + 1)}>Refresh</button>}</div>
    <Feedback loading={popular.loading} error={popular.error} retry={() => refresh((value) => value + 1)} />
    {songs.length ? rows(songs) : <p className="hint">No chart tracks are available right now.</p>}
  </section>;
}

function RadioPage({ station, rows }: { station?: string; rows: MusicRows }) {
  const selected = stations.find((item) => item.id === station);
  return <section className="screen">
    {selected ? <>
      <Link className="quiet-button back-button" href="/radio"><ArrowLeft size={16} /> All stations</Link>
      <h1>{selected.title}</h1><p className="hint">{selected.description}</p>
      {rows(selected.tracks.map((index) => featured[index]))}
    </> : <>
      <h1>Radio</h1>
      <div className="station-grid">{stations.map((item) =>
        <Link href={`/radio/${item.id}`} key={item.id} className={`station glass station-${item.id}`}>
          <Radio size={23} /><strong>{item.title}</strong><small>{item.description}</small><ArrowRight size={18} />
        </Link>)}</div>
    </>}
  </section>;
}

function PlaylistForm({ label, initial = "", submit }: { label: string; initial?: string; submit: (name: string) => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = String(new FormData(event.currentTarget).get("name") ?? "").trim();
    if (!name || busy) return;
    setBusy(true);
    setError("");
    try { await submit(name); }
    catch (error) { setError(describe(error)); }
    finally { setBusy(false); }
  }
  return <form className="playlist-form" onSubmit={save}>
    <input name="name" required maxLength={100} aria-label="Playlist name" placeholder="Playlist name" defaultValue={initial} />
    <button className="primary" disabled={busy}>{label}</button>
    {error && <Feedback error={error} />}
  </form>;
}

function Library({ account, mode, id, favorites, collection, mutation, refresh, rows, dataRevision }: {
  account?: Session;
  mode: string;
  id?: string;
  favorites: ReturnType<typeof useResource<YoutubePage>>;
  collection: ReturnType<typeof useResource<Collection>>;
  mutation: (path: string, method: string, body?: object) => Promise<void>;
  refresh: () => void;
  dataRevision: number;
  rows: MusicRows;
}) {
  const router = useRouter();
  const signedIn = !!account?.signed_in;
  const connected = !!account?.youtube_connected;
  const [revision, setRevision] = useState(0);
  const detail = useResource<PlaylistDetail>(signedIn && mode === "playlists" && id ? `library/playlists/${id}` : undefined, revision + dataRevision);
  const allYoutube = usePages(signedIn && connected && mode === "youtube" && !id, "youtube/playlists", playlists);
  const youtubeItems = usePages(signedIn && connected && mode === "youtube" && !!id, `youtube/playlist-items?playlist_id=${encodeURIComponent(id ?? "")}`, tracks);
  const [error, setError] = useState("");
  const [removing, setRemoving] = useState(false);
  async function change(path: string, method: string, body?: object) {
    await mutation(path, method, body);
    setRevision((value) => value + 1);
  }
  async function remove() {
    if (removing || !id || !window.confirm("Delete this Pulse playlist?")) return;
    setRemoving(true);
    setError("");
    try {
      await mutation(`library/playlists/${id}`, "DELETE");
      router.push("/library/playlists");
    } catch (error) { setError(describe(error)); }
    finally { setRemoving(false); }
  }
  const pages = id ? youtubeItems : allYoutube;
  const ytTitle = allYoutube.items.find((item) => item.id === id)?.title ?? "YouTube playlist";
  return <section className="screen">
    <h1>Your library</h1>
    <nav className="tabs" aria-label="Library collections">
      {([{ key: "liked", name: "Liked tracks", icon: Heart }, { key: "playlists", name: "Pulse playlists", icon: ListMusic },
        { key: "youtube", name: "YouTube", icon: MonitorPlay }]).map(({ key, name, icon: Icon }) =>
        <Link key={key} className={mode === key ? "active" : ""} aria-current={mode === key ? "page" : undefined} href={`/library/${key}`}><Icon size={16} /> {name}</Link>)}
    </nav>
    {!signedIn ? <div className="empty-state glass"><LibraryBig size={28} /><h2>Make it yours</h2>
      <p>Sign in to sync liked tracks and playlists.</p><Link className="primary" href="/login">Sign in</Link></div> : <>
      {mode === "liked" && <>
        <Feedback loading={favorites.loading} error={favorites.error} retry={refresh} />
        {favorites.data && (tracks(favorites.data).length ? rows(tracks(favorites.data)) :
          <p className="hint">{favorites.data.ids?.length ? "Connect YouTube to load your older liked tracks." : "Tap the heart next to a track to save it here."}</p>)}
      </>}
      {mode === "playlists" && !id && <>
        <PlaylistForm label="Create playlist" submit={async (name) => {
          const item = await api<PulsePlaylist>("library/playlists", { method: "POST", body: JSON.stringify({ name }) });
          refresh();
          router.push(`/library/playlists/${item.id}`);
        }} />
        <Feedback loading={collection.loading} error={collection.error} retry={refresh} />
        {!!collection.data?.items.length && <div className="album-grid section">{collection.data.items.map((item) =>
          <Link className="album-card" href={`/library/playlists/${item.id}`} key={item.id}>
            <Cover /><strong>{item.title}</strong><small>{item.count} tracks</small></Link>)}</div>}
      </>}
      {mode === "playlists" && id && <>
        <Link className="quiet-button back-button" href="/library/playlists"><ArrowLeft size={16} /> Pulse playlists</Link>
        <Feedback loading={detail.loading} error={detail.error || error} retry={() => setRevision((value) => value + 1)} />
        {detail.data && <>
          <div className="section-head"><h2>{detail.data.title}</h2>
            <button type="button" className="icon-button" aria-label="Delete playlist" disabled={removing} onClick={remove}><Trash2 size={17} /></button></div>
          <details><summary>Rename playlist</summary><PlaylistForm key={detail.data.title} label="Rename" initial={detail.data.title}
            submit={(name) => change(`library/playlists/${id}`, "PUT", { name })} /></details>
          {tracks(detail.data).length ? rows(tracks(detail.data), (track) => {
            void change(`library/playlists/${id}/tracks/${track.id}`, "DELETE").catch((error) => setError(describe(error)));
          }) : <p className="hint">Open a track and use Add to playlist to put it here.</p>}
        </>}
      </>}
      {mode === "youtube" && (!connected
        ? <div className="empty-state glass"><MonitorPlay size={29} /><h2>Open your YouTube playlists</h2><p>Connect YouTube in Settings to browse your own playlists.</p>
          <Link className="primary" href="/settings">Manage connections</Link></div>
        : <>
          {id && <><Link className="quiet-button back-button" href="/library/youtube"><ArrowLeft size={16} /> YouTube playlists</Link><h2>{ytTitle}</h2></>}
          <Feedback loading={pages.loading} error={pages.error} retry={pages.retry} />
          <Feedback error={pages.pageError} />
          {id ? rows(youtubeItems.items) : <div className="album-grid section">{allYoutube.items.map((item) =>
            <Link className="album-card" href={`/library/youtube/${item.id}`} key={item.id}>
              <Cover src={item.artwork} /><strong>{item.title}</strong><small>{item.count} tracks</small></Link>)}</div>}
          {!pages.loading && !pages.error && !pages.items.length && <p className="hint">{id ? "No tracks are available in this playlist." : "No YouTube playlists were found."}</p>}
          {pages.hasNext && <button className="quiet-button" disabled={pages.paging} onClick={pages.next}>Load more</button>}
        </>)}
    </>}
  </section>;
}

function TrackPanel({ track, account, collection, saved, save, add, close }: {
  track: Track; account?: Session; collection: PulsePlaylist[]; saved: boolean;
  save: () => void; add: (id: string) => Promise<void>; close: () => void;
}) {
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  async function addTrack(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const id = String(new FormData(event.currentTarget).get("playlist") ?? "");
    if (!id || busy) return;
    setBusy(true);
    try { await add(id); setStatus("Added to playlist."); }
    catch (error) { setStatus(describe(error)); }
    finally { setBusy(false); }
  }
  return <aside className="player-panel glass" aria-label="Track details">
    <div className="panel-label"><Disc3 size={18} /><span>Track details</span><button className="icon-button panel-close" onClick={close} aria-label="Close track details"><X size={18} /></button></div>
    <Cover src={track.artwork} className="current-cover" />
    <div className="now-title"><div><h2>{track.title}</h2><p>{track.artist}</p></div>
      <button className={`icon-button save ${saved ? "is-saved" : ""}`} aria-label={saved ? "Remove liked track" : "Like track"}
        aria-pressed={saved} onClick={save}><Heart size={22} fill={saved ? "currentColor" : "none"} /></button></div>
    {account?.signed_in && collection.length > 0 && <form className="track-playlist" onSubmit={addTrack}>
      <select aria-label="Select a playlist" name="playlist" required defaultValue=""><option disabled value="">Select playlist</option>
        {collection.map((item) => <option key={item.id} value={item.id}>{item.title}</option>)}</select>
      <button className="quiet-button" disabled={busy}><Plus size={16} /> Add</button>
    </form>}
    {account?.signed_in && collection.length === 0 && <Link className="quiet-button" href="/library/playlists"><Plus size={16} /> Create a playlist</Link>}
    {status && <p className="hint" role="status">{status}</p>}
  </aside>;
}

export default function Pulse({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const [section, mode, id] = pathname.split("/").filter(Boolean);
  const router = useRouter();
  const [account, setAccount] = useState<Session>();
  const [sessionError, setSessionError] = useState("");
  const [notice, setNotice] = useState("");
  const [revision, setRevision] = useState(0);
  const [current, setCurrent] = useState<Track>();
  const [cancelled, setCancelled] = useState(false);
  const pending = useRef(new Set<string>());
  const authenticated = account?.signed_in === true;
  const youtubeConnected = account?.youtube_connected === true && account.google_available;
  const favorites = useResource<YoutubePage>(authenticated ? "favorites" : undefined, revision);
  const collection = useResource<Collection>(authenticated ? "library/playlists" : undefined, revision);
  const saved = new Set(favorites.data?.ids ?? []);
  useEffect(() => {
    setCancelled(new URLSearchParams(window.location.search).get("auth") === "cancelled");
  }, [pathname]);
  useEffect(() => {
    const controller = new AbortController();
    api<Session>("session", { signal: controller.signal }).then(setAccount).catch((error) => {
      if (!controller.signal.aborted) setSessionError(describe(error));
    });
    return () => controller.abort();
  }, []);

  async function reload() {
    setAccount(await api<Session>("session"));
    setSessionError("");
  }
  const refresh = () => setRevision((value) => value + 1);
  async function mutation(path: string, method: string, body?: object) {
    await api(path, { method, body: body ? JSON.stringify(body) : undefined });
    refresh();
  }
  async function toggleSave(item: Track) {
    if (!authenticated) { router.push("/login"); return; }
    if (pending.current.has(item.id) || favorites.loading) return;
    pending.current.add(item.id);
    setNotice("");
    try {
      const removing = saved.has(item.id);
      await mutation(`favorites/${item.id}`, removing ? "DELETE" : "PUT", removing ? undefined : { title: item.title, artist: item.artist });
    } catch (error) { setNotice(describe(error)); }
    finally { pending.current.delete(item.id); }
  }

  const rows: MusicRows = (items, remove) => <ul className="track-list">{Array.from(new Map(items.map((item) => [item.id, item])).values()).map((item) =>
    <li className="track-row" key={item.id}>
      <button className="track-open" onClick={() => setCurrent(item)}><Cover src={item.artwork} />
        <span className="track-text"><strong>{item.title}</strong><small>{item.artist}</small></span></button>
      <button className={`icon-button ${saved.has(item.id) ? "is-saved" : ""}`} aria-label={`Like ${item.title}`} aria-pressed={saved.has(item.id)}
        disabled={favorites.loading} onClick={() => toggleSave(item)}><Heart size={18} fill={saved.has(item.id) ? "currentColor" : "none"} /></button>
      {remove && <button className="icon-button" aria-label={`Remove ${item.title} from playlist`} onClick={() => remove(item)}><X size={17} /></button>}
    </li>)}</ul>;
  const nav = navigation.map(({ href, title, icon: Icon }) => {
    const active = href === "/" ? pathname === "/" : pathname === href || pathname.startsWith(`${href}/`);
    return <Link key={href} className={`nav-link ${active ? "selected" : ""}`} aria-current={active ? "page" : undefined} href={href}><Icon size={20} /><span>{title}</span></Link>;
  });
  const showPanel = current && !["login", "signup", "player", "settings"].includes(section);
  let page: ReactNode = null;
  switch (section) {
    case undefined:
      page = <>
        <section className="hero">
          <div className="hero-art" style={{ backgroundImage: `url("${featured[0].artwork}")` }} />
          <div className="hero-copy">
            <p className="kicker"><AudioLines size={17} /> Pulse radio</p><h1>After Hours</h1>
            <p>Synth-pop, late-night indie and a little disco.</p>
            <Link className="primary" href="/radio/night"><Radio size={17} /> Explore station</Link>
          </div>
        </section>
        <section className="section">
          <div className="section-head"><h2>On rotation</h2><Link href="/new">Browse music <ArrowRight size={15} /></Link></div>
          <div className="album-grid">{featured.slice(0, 4).map((item) =>
            <button className="album-card" key={item.id} onClick={() => setCurrent(item)}><Cover src={item.artwork} /><strong>{item.title}</strong><small>{item.artist}</small></button>)}</div>
        </section>
        <section className="section"><div className="section-head"><h2>Tracks</h2></div>{rows(featured)}</section>
      </>;
      break;
    case "new": if (!mode) page = <NewMusic connected={youtubeConnected} rows={rows} />; break;
    case "radio": if (!id && (!mode || stations.some((station) => station.id === mode))) page = <RadioPage station={mode} rows={rows} />; break;
    case "search": if (!mode) page = <SearchPage connected={youtubeConnected} rows={rows} />; break;
    case "library": if ((!mode || ["liked", "playlists", "youtube"].includes(mode)) && (!id || (/^[\w-]{1,150}$/.test(id) && mode !== "liked"))) page = <Library key={pathname} account={account} mode={mode ?? "liked"} id={id}
      favorites={favorites} collection={collection} mutation={mutation} refresh={refresh} dataRevision={revision} rows={rows} />; break;
    case "player": if (!mode) page = <SpotifyPage account={account} />; break;
    case "login":
    case "signup":
      if (!mode) page = <AccountPage mode={section} account={account} cancelled={cancelled} complete={async () => { await reload(); router.replace("/"); }} />;
      break;
    case "settings":
      if (!mode) page = <Settings account={account} reload={reload} cancelled={cancelled} signedOut={async () => {
        await reload(); setCurrent(undefined); refresh(); router.replace("/");
      }} />;
      break;
  }
  return <div className={`app-shell ${showPanel ? "with-panel" : ""}`}>
    <a className="skip-link" href="#main">Skip to content</a>
    <aside className="sidebar glass">
      <Link className="brand" href="/" aria-label="Pulse home"><img src="/pulse.png" alt="" /> <span>Pulse</span></Link>
      <nav aria-label="Main navigation">{nav}</nav>
      <div className="sidebar-label">Your collection</div>
      <nav aria-label="Collection shortcuts">
        <Link className="nav-link" href="/library/liked"><Heart size={18} /> Liked tracks</Link>
        <Link className="nav-link" href="/library/playlists"><ListMusic size={18} /> Playlists</Link>
        <Link className="nav-link" href="/player"><Headphones size={18} /> Spotify player</Link>
      </nav>
      <Link className="nav-link sidebar-footer" href="/settings"><Settings2 size={18} /> Settings</Link>
    </aside>
    <main id="main" className="content">
      <header className="topbar">
        <Link className="brand compact-brand" href="/" aria-label="Pulse home"><img src="/pulse.png" alt="" /><span>Pulse</span></Link>
        <Link className="search-shortcut glass" href="/search" aria-label="Search music"><Search size={17} /><span>Search music</span></Link>
        <div className="top-actions">
          <Link className="icon-button glass" aria-label="Spotify player" href="/player"><Headphones size={18} /></Link>
          <Link className="account-button glass" aria-label={authenticated ? "Account settings" : "Sign in"} href={authenticated ? "/settings" : "/login"}>
            {authenticated ? <span className="avatar">{(account.name || "P").charAt(0).toUpperCase()}</span> : <LogIn size={17} />}
            <span>{authenticated ? account.name || "Account" : "Sign in"}</span>
          </Link>
        </div>
      </header>
      {sessionError && <Feedback error={sessionError} retry={() => void reload().catch((error) => setSessionError(describe(error)))} />}
      {notice && <div className="notice" role="status"><CircleAlert size={18} /><span>{notice}</span>
        <button className="icon-button" aria-label="Dismiss message" onClick={() => setNotice("")}><X size={17} /></button></div>}
      {page}
      {children}
    </main>
    {showPanel && <TrackPanel key={current.id} track={current} account={account} collection={collection.data?.items ?? []} saved={saved.has(current.id)}
      save={() => void toggleSave(current)} close={() => setCurrent(undefined)}
      add={(playlist) => mutation(`library/playlists/${playlist}/tracks/${current.id}`, "PUT", { title: current.title, artist: current.artist })} />}
    <nav className="mobile-nav glass" aria-label="Mobile navigation">{nav}</nav>
  </div>;
}
