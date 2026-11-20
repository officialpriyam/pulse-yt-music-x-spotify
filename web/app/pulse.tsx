"use client";

import {
  AudioLines, CircleAlert, Disc3, Heart, House,
  LibraryBig, ListMusic, LogIn, LogOut, Music2,
  Search, X,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { api, featured, playlists, tracks, type Playlist, type Track, type YoutubePage } from "./catalog";

type Screen = "Home" | "Search" | "Library";
type Session = { google_connected: boolean; name?: string; configured?: boolean };
type Page = { songs: Track[]; next?: string };
type Collection = { playlists: Playlist[]; next?: string };
const navigation = [{ title: "Home", icon: House }, { title: "Search", icon: Search }, { title: "Library", icon: LibraryBig }] as const;

function describe(error: unknown) {
  return error instanceof Error ? error.message : "Something went wrong. Try again.";
}

function Cover({ src, className = "" }: { src: string; className?: string }) {
  const [failed, setFailed] = useState("");
  return <div className={`cover ${className}`}>
    {src && failed !== src ? <img src={src} alt="" onError={() => setFailed(src)} /> : <Music2 aria-hidden />}
  </div>;
}

function TrackRows({ songs, saved, onSelect, onSave }: {
  songs: Track[];
  saved: Set<string>;
  onSelect: (track: Track) => void;
  onSave: (track: Track) => void;
}) {
  return <ul className="track-list">
    {songs.map((song) => <li className="track-row" key={song.id}>
      <button className="track-open" onClick={() => onSelect(song)}>
        <Cover src={song.artwork} />
        <span className="track-text"><strong>{song.title}</strong><small>{song.artist}</small></span>

      </button>
      <button type="button" className={`icon-button save ${saved.has(song.id) ? "is-saved" : ""}`}
        aria-label={saved.has(song.id) ? `Remove ${song.title} from library` : `Save ${song.title}`}
        aria-pressed={saved.has(song.id)} onClick={() => onSave(song)}>
        <Heart size={18} fill={saved.has(song.id) ? "currentColor" : "none"} />
      </button>
    </li>)}
  </ul>;
}

export default function Pulse() {
  const [screen, setScreen] = useState<Screen>("Home");
  const [account, setAccount] = useState<Session>();
  const [accountMenu, setAccountMenu] = useState(false);
  const [notice, setNotice] = useState("");
  const [query, setQuery] = useState("");
  const [searched, setSearched] = useState("");
  const [searchPage, setSearchPage] = useState<Page>({ songs: [] });
  const [collection, setCollection] = useState<Collection>({ playlists: [] });
  const [selectedPlaylist, setSelectedPlaylist] = useState<Playlist>();
  const [playlistPage, setPlaylistPage] = useState<Page>({ songs: [] });
  const [favoriteTracks, setFavoriteTracks] = useState<Track[]>([]);
  const [saved, setSaved] = useState<Set<string>>(new Set());
  const [libraryMode, setLibraryMode] = useState<"saved" | "playlists">("saved");
  const [busy, setBusy] = useState(false);
  const [current, setCurrent] = useState(featured[0]);
  const searchInput = useRef<HTMLInputElement | null>(null);
  const savingTracks = useRef(new Set<string>());
  const requestVersion = useRef(0);
  const connected = account?.google_connected === true;

  useEffect(() => {
    const controller = new AbortController();
    api<Session>("session", { signal: controller.signal }).then(setAccount).catch((error) => {
      if (!controller.signal.aborted) setNotice(describe(error));
    });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    if (!connected) return;
    const controller = new AbortController();
    api<YoutubePage>("favorites", { signal: controller.signal }).then((page) => {
      setFavoriteTracks(tracks(page));
      setSaved(new Set(page.ids ?? []));
    }).catch((error) => {
      if (!controller.signal.aborted) setNotice(describe(error));
    });
    return () => controller.abort();
  }, [connected]);

  useEffect(() => {
    if (screen === "Search") searchInput.current?.focus();
  }, [screen]);

  function go(next: Screen) {
    requestVersion.current += 1;
    setBusy(false);
    setNotice("");
    setScreen(next);
  }

  function login() {
    if (account?.configured === false) {
      setNotice("Google login is unavailable until the Pulse API is configured.");
      return;
    }
    window.location.assign("/auth/google/login");
  }

  async function logout() {
    try {
      const response = await fetch("/auth/logout", { method: "POST", headers: { "X-Pulse-Request": "1" } });
      if (!response.ok) throw new Error("Could not sign out. Try again.");
      requestVersion.current += 1;
      setAccount({ google_connected: false, configured: true });
      setAccountMenu(false);
      setFavoriteTracks([]);
      setSaved(new Set());
      setCollection({ playlists: [] });
      setLibraryMode("saved");
      setSelectedPlaylist(undefined);
      setPlaylistPage({ songs: [] });
      setSearchPage({ songs: [] });
      setSearched("");
      setQuery("");
      setBusy(false);
      setNotice("");
    } catch (error) { setNotice(describe(error)); }
  }

  async function fetchPage<T>(path: string, apply: (result: T) => void) {
    const version = ++requestVersion.current;
    setBusy(true);
    setNotice("");
    try {
      const result = await api<T>(path);
      if (version === requestVersion.current) apply(result);
    } catch (error) {
      if (version === requestVersion.current) setNotice(describe(error));
    } finally {
      if (version === requestVersion.current) setBusy(false);
    }
  }

  function searchCatalog(event?: FormEvent, next?: string) {
    event?.preventDefault();
    const q = (next ? searched : query).trim();
    if (!q) return;
    if (!next) {
      setSearched(q);
      setSearchPage({ songs: [] });
    }
    if (!connected) {
      setSearchPage({ songs: featured.filter((song) => `${song.title} ${song.artist}`.toLowerCase().includes(q.toLowerCase())) });
      return;
    }
    const params = new URLSearchParams({ q });
    if (next) params.set("page_token", next);
    void fetchPage<YoutubePage>(`youtube/search?${params}`, (page) => {
      setSearchPage((previous) => ({ songs: next ? [...previous.songs, ...tracks(page)] : tracks(page), next: page.nextPageToken }));
    });
  }

  function showPlaylists(next?: string) {
    if (!connected) { login(); return; }
    setLibraryMode("playlists");
    setSelectedPlaylist(undefined);
    const params = next ? `?page_token=${encodeURIComponent(next)}` : "";
    void fetchPage<YoutubePage>(`youtube/playlists${params}`, (page) => {
      setCollection((previous) => ({ playlists: next ? [...previous.playlists, ...playlists(page)] : playlists(page), next: page.nextPageToken }));
    });
  }

  function showTracks(list: Playlist, next?: string) {
    setSelectedPlaylist(list);
    if (!next) setPlaylistPage({ songs: [] });
    const params = new URLSearchParams({ playlist_id: list.id });
    if (next) params.set("page_token", next);
    void fetchPage<YoutubePage>(`youtube/playlist-items?${params}`, (page) => {
      setPlaylistPage((previous) => ({ songs: next ? [...previous.songs, ...tracks(page)] : tracks(page), next: page.nextPageToken }));
    });
  }

  async function save(song: Track) {
    if (!connected) { login(); return; }
    if (savingTracks.current.has(song.id)) return;
    savingTracks.current.add(song.id);
    const remove = saved.has(song.id);
    try {
      await api(`favorites/${song.id}`, { method: remove ? "DELETE" : "PUT" });
      setSaved((before) => {
        const updated = new Set(before);
        if (remove) updated.delete(song.id);
        else updated.add(song.id);
        return updated;
      });
      setFavoriteTracks((before) => remove ? before.filter((track) => track.id !== song.id) :
        [song, ...before.filter((track) => track.id !== song.id)]);
    } catch (error) { setNotice(describe(error)); }
    finally { savingTracks.current.delete(song.id); }
  }

  const isLibrary = screen === "Library";
  const activeList = selectedPlaylist ? playlistPage.songs : favoriteTracks;
  const rendered = (songs: Track[]) => <TrackRows songs={Array.from(new Map(songs.map((song) => [song.id, song])).values())}
    saved={saved} onSelect={setCurrent} onSave={save} />;

  return <div className="app-shell">
    <aside className="sidebar glass">
      <button className="brand" onClick={() => go("Home")} aria-label="Pulse home">
        <img src="/pulse.png" alt="" /><span>PULSE</span>
      </button>
      <nav aria-label="Main navigation">
        {navigation.map(({ title, icon: Icon }) => <button key={title} className={`nav-link ${screen === title ? "selected" : ""}`}
          aria-current={screen === title ? "page" : undefined} onClick={() => go(title)}><Icon size={20} /><span>{title}</span></button>)}
      </nav>
      <div className="sidebar-label">Your collection</div>
      <button className="nav-link collection-item" onClick={() => { go("Library"); setLibraryMode("saved"); setSelectedPlaylist(undefined); }}>
        <span className="collection-icon"><Heart size={16} /></span><span>Liked tracks</span>
      </button>
      <button className="nav-link collection-item" onClick={() => { go("Library"); showPlaylists(); }}>
        <span className="collection-icon tint"><ListMusic size={17} /></span><span>YouTube playlists</span>
      </button>
      <div className="sidebar-footer"><AudioLines size={15} /> YouTube Music</div>
    </aside>

    <main className="content">
      <header className="topbar">
        <button className="brand compact-brand" onClick={() => go("Home")} aria-label="Pulse home"><img src="/pulse.png" alt="" /><span>PULSE</span></button>
        <button className="search-shortcut glass" aria-label="Search music" onClick={() => go("Search")}><Search size={17} /> <span>Search music</span></button>
        <div className="account">
          {connected ? <button className="account-button glass" aria-expanded={accountMenu} onClick={() => setAccountMenu(!accountMenu)}>
            <span className="avatar">{(account?.name || "P").charAt(0).toUpperCase()}</span><span>{account?.name || "Google account"}</span>
          </button> : <button className="account-button glass" onClick={login}><LogIn size={16} /> <span>Connect Google</span></button>}
          {connected && accountMenu && <div className="account-menu glass"><button onClick={logout}><LogOut size={17} /> Sign out</button></div>}
        </div>
      </header>

      {notice && <div className="notice" role="status"><CircleAlert size={18} /><span>{notice}</span>
        <button className="icon-button" aria-label="Dismiss message" onClick={() => setNotice("")}><X size={17} /></button></div>}

      {screen === "Home" && <>
        <section className="hero">
          <div className="hero-art" style={{ backgroundImage: `url("${featured[0].artwork}")` }} />
          <div className="hero-copy">
            <p className="kicker"><AudioLines size={17} /> Pulse mix</p>
            <h1>After Hours</h1>
            <p>Synth-pop, late-night indie and a little disco.</p>
            <button className="primary" onClick={() => document.getElementById("track-list")?.scrollIntoView({ block: "start" })}><ListMusic size={16} /> Browse tracks</button>
          </div>
        </section>

        <section className="section">
          <div className="section-head"><h2>On rotation</h2><span>From the Pulse mix</span></div>
          <div className="album-grid">
            {featured.slice(0, 4).map((song) => <button className="album-card" key={song.id} onClick={() => setCurrent(song)}>
              <Cover src={song.artwork} />
              <strong>{song.title}</strong><small>{song.artist}</small>
            </button>)}
          </div>
        </section>

        <section className="section tracks-section" id="track-list">
          <div className="section-head"><h2>Track list</h2></div>
          {rendered(featured)}
        </section>
      </>}

      {screen === "Search" && <section className="screen">
        <h1>Search</h1>
        <form className="search-form glass" onSubmit={searchCatalog}>
          <Search size={21} /><input ref={searchInput} aria-label="Search music" value={query} maxLength={120}
            onChange={(event) => setQuery(event.target.value)} placeholder="Tracks, artists and music videos" />
          <button className="primary" type="submit" disabled={busy || !query.trim()}>Search</button>
        </form>
        {!connected && <p className="hint">Search the Pulse mix, or <button onClick={login}>connect Google</button> to search YouTube.</p>}
        {searched && <h2 className="results-heading">Results for “{searched}”</h2>}
        {searchPage.songs.length > 0 && rendered(searchPage.songs)}
        {busy && <p className="hint" role="status">Searching…</p>}
        {!busy && searched && searchPage.songs.length === 0 && !notice && <p className="hint">No tracks found.</p>}
        {searchPage.next && <button className="quiet-button" disabled={busy} onClick={() => searchCatalog(undefined, searchPage.next)}>Load more</button>}
      </section>}

      {isLibrary && <section className="screen">
        <h1>Your library</h1>
        <div className="tabs">
          <button className={libraryMode === "saved" ? "active" : ""} onClick={() => {
            requestVersion.current += 1; setBusy(false); setNotice(""); setLibraryMode("saved"); setSelectedPlaylist(undefined);
          }}><Heart size={16} /> Liked tracks</button>
          <button className={libraryMode === "playlists" ? "active" : ""} onClick={() => showPlaylists()}><ListMusic size={18} /> YouTube playlists</button>
        </div>
        {!connected && <div className="empty-state glass"><LibraryBig size={30} /><h2>Bring your music with you</h2>
          <p>Connect Google to open your YouTube playlists and save tracks to Pulse.</p>
          <button className="primary" onClick={login}><LogIn size={16} /> Connect Google</button></div>}
        {connected && libraryMode === "saved" && <>
          {favoriteTracks.length ? rendered(favoriteTracks) : <p className="hint">Tap the heart next to a track to save it here.</p>}
        </>}
        {connected && libraryMode === "playlists" && (selectedPlaylist ?
          <div>
            <button className="quiet-button back-button" onClick={() => {
              requestVersion.current += 1; setBusy(false); setSelectedPlaylist(undefined);
            }}>All playlists</button>
            <h2>{selectedPlaylist.title}</h2>
            {activeList.length > 0 && rendered(activeList)}
            {!busy && activeList.length === 0 && !notice && <p className="hint">No tracks are available in this playlist.</p>}
            {playlistPage.next && <button className="quiet-button" disabled={busy} onClick={() => showTracks(selectedPlaylist, playlistPage.next)}>Load more</button>}
          </div> : <>
            {collection.playlists.length > 0 && <div className="album-grid playlists-grid">
              {collection.playlists.map((list) => <button key={list.id} className="album-card" onClick={() => showTracks(list)}>
                <Cover src={list.artwork} /><strong>{list.title}</strong><small>{list.count} videos</small>
              </button>)}
            </div>}
            {!busy && collection.playlists.length === 0 && !notice && <p className="hint">This account has no YouTube playlists.</p>}
            {collection.next && <button className="quiet-button" disabled={busy} onClick={() => showPlaylists(collection.next)}>Load more</button>}
          </>)}
        {busy && <p className="hint" role="status">Loading library…</p>}
      </section>}
    </main>

    <aside className="player-panel glass" aria-label="Track details">
      <div className="panel-label"><Disc3 size={18} /> <span>Track details</span></div>
      <Cover src={current.artwork} className="current-cover" />
      <div className="now-title"><div><h2>{current.title}</h2><p>{current.artist}</p></div>
        <button className={`icon-button save ${saved.has(current.id) ? "is-saved" : ""}`} onClick={() => save(current)}
          aria-label="Save selected track" aria-pressed={saved.has(current.id)}>
          <Heart size={20} fill={saved.has(current.id) ? "currentColor" : "none"} />
        </button></div>
    </aside>

    <nav className="mobile-nav glass" aria-label="Mobile navigation">
      {navigation.map(({ title, icon: Icon }) => <button key={title} className={screen === title ? "selected" : ""}
        aria-current={screen === title ? "page" : undefined} onClick={() => go(title)}><Icon size={20} />{title}</button>)}
    </nav>
  </div>;
}
