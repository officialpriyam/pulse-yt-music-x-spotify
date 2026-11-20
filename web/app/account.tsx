"use client";

import Link from "next/link";
import { useState, type FormEvent } from "react";
import { ArrowRight, AudioLines, CircleCheck, Disc3, Headphones, LogOut, MessagesSquare, MonitorPlay } from "lucide-react";
import { api, describe, type Session } from "./catalog";

function CredentialsForm({ mode, done }: { mode: "login" | "signup" | "credentials"; done: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;
    const form = new FormData(event.currentTarget);
    const body = JSON.stringify({ username: form.get("username"), password: form.get("password") });
    setBusy(true);
    setError("");
    try {
      await api(`/auth/${mode}`, { method: "POST", body });
      await done();
    } catch (error) {
      setError(describe(error));
    } finally {
      setBusy(false);
    }
  }
  return <form className="credentials" onSubmit={submit}>
    <label>Username<input name="username" required autoComplete="username" minLength={3} maxLength={32} pattern="[A-Za-z0-9_]{3,32}" /></label>
    <label>Password<input name="password" type="password" required minLength={12} maxLength={128}
      autoComplete={mode === "login" ? "current-password" : "new-password"} /></label>
    {mode !== "login" && <small>Use at least 12 characters. Save your password in a password manager.</small>}
    {error && <p className="form-error" role="alert">{error}</p>}
    <button type="submit" className="primary" disabled={busy}>
      {busy ? "Please wait…" : { login: "Sign in", signup: "Create account", credentials: "Add password sign-in" }[mode]}
      <ArrowRight size={17} />
    </button>
  </form>;
}

export function AccountPage({ mode, account, complete, cancelled }: {
  mode: "login" | "signup";
  account?: Session;
  complete: () => Promise<void>;
  cancelled: boolean;
}) {
  const login = mode === "login";
  return <section className="screen auth-screen">
    <div className="auth-card glass">
      <img src="/pulse.png" alt="" width={52} height={52} />
      <h1>{login ? "Sign in to Pulse" : "Create your Pulse account"}</h1>
      <p>{login ? "Open your saved tracks and playlists." : "Save tracks, build playlists and connect your music accounts."}</p>
      {cancelled && <p className="form-error" role="status">Sign-in was cancelled. You can try again.</p>}
      {account?.signed_in ? <Link className="primary" href="/settings">Manage your account <ArrowRight size={17} /></Link>
        : account?.configured ? <CredentialsForm key={mode} mode={mode} done={complete} />
        : <p role="status">{account ? "Account sign-in is currently unavailable." : "Checking account availability…"}</p>}
      {!account?.signed_in && <>
        {(account?.google_available || account?.discord_available) && <div className="provider-choices">
          <span>Or continue with</span>
          {account.google_available && <a className="quiet-button" href="/auth/google/login"><b aria-hidden>G</b> Google</a>}
          {account.discord_available && <a className="quiet-button" href="/auth/discord/login"><MessagesSquare size={17} /> Discord</a>}
        </div>}
        <p>{login ? "New to Pulse?" : "Already have an account?"} <Link href={login ? "/signup" : "/login"}>{login ? "Create an account" : "Sign in"}</Link></p>
      </>}
    </div>
  </section>;
}

export function Settings({ account, reload, signedOut, cancelled }: {
  account?: Session;
  reload: () => Promise<void>;
  signedOut: () => Promise<void>;
  cancelled: boolean;
}) {
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  async function action(path: string, complete: () => Promise<void>) {
    if (busy) return;
    setBusy(path);
    setError("");
    try {
      await api(`/auth/${path}`, { method: "POST" });
      await complete();
    } catch (error) { setError(describe(error)); }
    finally { setBusy(""); }
  }
  const services = [
    { id: "google", title: "Google", description: "Use Google to sign in to Pulse.", icon: AudioLines, available: account?.google_available, linked: account?.google_connected, music: false },
    { id: "discord", title: "Discord", description: "Use Discord to sign in to Pulse.", icon: MessagesSquare, available: account?.discord_available, linked: account?.discord_linked, music: false },
    { id: "youtube", title: "YouTube", description: "Search YouTube music and browse your playlists.", icon: MonitorPlay, available: account?.google_available, linked: account?.youtube_connected, music: true },
    { id: "spotify", title: "Spotify", description: "Play audio in your browser with Spotify Premium.", icon: Headphones, available: account?.spotify_available, linked: account?.spotify_connected, music: true },
  ];
  return <section className="screen">
    <h1>Settings</h1>
    {error && <p className="form-error" role="alert">{error}</p>}
    {cancelled && <p className="form-error" role="status">Connection cancelled. You can try again.</p>}
    <div className="settings-card glass">
      <div className="settings-profile">
        <span className="avatar">{(account?.name || "P").charAt(0).toUpperCase()}</span>
        <div><h2>{account?.signed_in ? account.name || "Your account" : "Pulse account"}</h2>
          {account?.username && <small>@{account.username}</small>}</div>
        {account?.signed_in
          ? <button className="quiet-button" disabled={!!busy} onClick={() => action("logout", signedOut)}><LogOut size={17} /> Sign out</button>
          : <Link className="quiet-button" href="/login">Sign in</Link>}
      </div>
      {account?.signed_in && !account.username && <details><summary>Add username and password</summary><CredentialsForm mode="credentials" done={reload} /></details>}
    </div>
    {account?.signed_in && <section className="section settings-card glass">
      <h2>Connections</h2>
      {services.map(({ id, title, description, icon: Icon, available, linked, music }) =>
        <div className="connection" key={id}><Icon size={20} />
          <div><strong>{title}</strong><small>{description}</small></div>
          {linked
            ? music ? <button disabled={!!busy} className="quiet-button" onClick={() => action(`${id}/disconnect`, reload)}>Disconnect</button>
              : <span className="linked"><CircleCheck size={16} /> Linked</span>
            : available ? <a className="quiet-button" href={`/auth/${id}/login?connect=true`}>Connect</a> : <small>Unavailable</small>}
        </div>
      )}
    </section>}
    <section className="section settings-card glass">
      <h2>About Pulse</h2>
      <div className="connection"><Disc3 size={20} /><div><strong>Pulse for Android</strong><small>Developed by priyx</small></div>
        <a className="quiet-button" href="https://github.com/officialpriyam/pulse-yt-music-x-spotify/releases/latest">Get the APK</a></div>
    </section>
  </section>;
}
