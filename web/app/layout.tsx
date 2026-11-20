import type { Metadata } from "next";
import Pulse from "./pulse";
import "./globals.css";

export const metadata: Metadata = {
  title: { default: "Pulse", template: "%s · Pulse" },
  description: "Browse music, build your library and connect your music services.",
  icons: { icon: "/pulse.png" },
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return <html lang="en"><body><Pulse>{children}</Pulse></body></html>;
}
