"use client";

import Link from "next/link";
import { Disc3, Pause, Play, SkipBack, SkipForward, Volume2 } from "lucide-react";
import { useRef, useState } from "react";
import type { Track } from "./catalog";

export function usePulseAudio() {
  const media = useRef<HTMLAudioElement>(null);
  const queue = useRef<Track[]>([]);
  const index = useRef(0);
  const request = useRef(0);
  const [track, setTrack] = useState<Track>();
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  const [volume, setVolume] = useState(1);
  const [error, setError] = useState("");

  function start(item: Track, items: Track[] = [item]) {
    const audio = media.current;
    if (!audio || !item.audio) return;
    const playable = [...new Map(items.filter((item) => item.audio).map((item) => [item.id, item])).values()];
    if (playable.some((candidate) => candidate.id === item.id)) {
      queue.current = playable;
    } else {
      queue.current = [item];
    }
    index.current = queue.current.findIndex((candidate) => candidate.id === item.id);
    const source = new URL(item.audio, window.location.origin).href;
    if (audio.src !== source || audio.error) {
      audio.src = source;
      setPosition(0);
      setDuration(0);
    }
    setTrack(item);
    play(audio);
  }

  function play(audio: HTMLAudioElement) {
    const attempt = ++request.current;
    setError("");
    void audio.play().catch((failure: unknown) => {
      if (attempt !== request.current || (failure instanceof DOMException && failure.name === "AbortError")) return;
      setError("Couldn’t start this track. Press Play to retry or choose another song.");
    });
  }

  function toggle() {
    const audio = media.current;
    if (!audio || !track) return;
    if (!audio.paused) audio.pause();
    else if (audio.error) start(track, queue.current);
    else play(audio);
  }

  function step(direction: number) {
    const next = queue.current[index.current + direction];
    if (next) start(next, queue.current);
  }

  function previous() {
    if (media.current && media.current.currentTime > 3) seek(0);
    else step(-1);
  }

  function seek(seconds: number) {
    if (media.current && Number.isFinite(media.current.duration)) {
      media.current.currentTime = Math.max(0, Math.min(seconds, media.current.duration));
      setPosition(media.current.currentTime);
    }
  }

  return {
    media, start, toggle, previous, seek, track, playing, position, duration, volume, error,
    hasNext: index.current < queue.current.length - 1,
    hasPrevious: index.current > 0 || position > 3,
    next: () => step(1),
    setVolume: (value: number) => {
      if (media.current) media.current.volume = value;
      setVolume(value);
    },
    onPlay: () => setPlaying(true),
    onPause: () => setPlaying(false),
    onEnded: () => { setPlaying(false); step(1); },
    onTime: () => setPosition(media.current?.currentTime || 0),
    onDuration: () => {
      const value = media.current?.duration;
      if (typeof value === "number" && Number.isFinite(value)) setDuration(value);
    },
    onError: () => {
      if (media.current?.error) {
        setPlaying(false);
        setError("This track couldn’t load. Press Play to retry.");
      }
    },
  };
}

function time(seconds: number) {
  const value = Math.floor(seconds);
  return `${Math.floor(value / 60)}:${String(value % 60).padStart(2, "0")}`;
}

export function PulseAudio({ player, visible }: { player: ReturnType<typeof usePulseAudio>; visible: boolean }) {
  return <>
    <audio ref={player.media} preload="none" onPlay={player.onPlay} onPause={player.onPause}
      onEnded={player.onEnded} onError={player.onError} onTimeUpdate={player.onTime} onDurationChange={player.onDuration} />
    {visible && player.track && <section className="audio-bar glass" aria-label="Pulse audio player">
      <Link className="audio-title" href="/player">
        <span className={`cover ${player.track.id}`}><Disc3 size={23} /></span>
        <span className="track-text"><strong>{player.track.title}</strong><small>{player.track.artist}</small></span>
      </Link>
      <div className="audio-transport">
        <button className="icon-button" aria-label="Previous track" disabled={!player.hasPrevious} onClick={player.previous}><SkipBack size={18} /></button>
        <button className="primary audio-toggle" aria-label={player.playing ? "Pause" : "Play"} onClick={player.toggle}>
          {player.playing ? <Pause size={18} fill="currentColor" /> : <Play size={18} fill="currentColor" />}
        </button>
        <button className="icon-button" aria-label="Next track" disabled={!player.hasNext} onClick={player.next}><SkipForward size={18} /></button>
      </div>
      <div className="audio-progress"><small>{time(player.position)}</small>
        <input type="range" aria-label="Playback position" min={0} max={player.duration || 1} step={0.1}
          value={player.position} disabled={!player.duration} onChange={(event) => player.seek(Number(event.target.value))} />
        <small>{time(player.duration)}</small>
      </div>
      <div className="audio-volume"><Volume2 size={16} />
        <input type="range" aria-label="Volume" min={0} max={1} step={0.02} value={player.volume} onChange={(event) => player.setVolume(Number(event.target.value))} />
      </div>
      {player.error && <p role="alert" className="audio-error">{player.error}</p>}
    </section>}
  </>;
}
