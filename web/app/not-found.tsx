import Link from "next/link";

export default function NotFound() {
  return <section className="screen"><h1>Page unavailable</h1><Link href="/" className="primary">Back to Home</Link></section>;
}
