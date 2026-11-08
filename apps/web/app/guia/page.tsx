import type { Metadata } from "next";

export const metadata: Metadata = { title: "Guía" };

export default function GuidePage() {
  return (
    <iframe
      src="/guia/doc"
      title="Guía de uso de AgentSI"
      className="block h-[calc(100dvh-7.5rem)] md:h-[calc(100dvh-4rem)] w-full rounded-lg border border-border"
    />
  );
}
