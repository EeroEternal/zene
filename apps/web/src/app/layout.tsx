import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Zene Agent Cloud",
  description: "Web UI for Flue Agents",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
