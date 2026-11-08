import { notFound } from "next/navigation";
import { CallPlayer, type Segment } from "@/components/call-player";
import { Crumbs, Empty } from "@/components/bits";
import { ListenedButton } from "@/components/listened-button";
import { loadInbox } from "@/lib/inbox-data";
import { findProduct } from "@/lib/registry";
import { sortRoles } from "@/lib/roles";
import { audioFile, getMinutes, getTimings } from "@/lib/workspace";

export const dynamic = "force-dynamic";

export default async function CallPage({ params }: { params: Promise<{ product: string; id: string }> }) {
  const { product: pid, id } = await params;
  const product = findProduct(pid);
  const minutes = product ? await getMinutes(pid, id) : null;
  if (!product || !minutes) notFound();

  const timings = getTimings(pid, id);
  const timed = timings.length > 0;
  const segments: Segment[] = timed
    ? timings
    : (minutes.script ?? []).map((s, idx) => ({ idx, role: s.role, text: s.text, start: 0, end: 0 }));
  const hasAudio = !!audioFile(pid, id);
  const heard = !!(await loadInbox()).minutes.find((m) => m.id === id)?.heard;
  const participants = sortRoles(Array.from(new Set([...product.roles, ...segments.map((s) => s.role)])));

  return (
    <div>
      <Crumbs
        items={[
          { label: "Productos", href: "/" },
          { label: product.name, href: `/p/${pid}` },
          { label: "Minutes", href: `/p/${pid}/m/${id}` },
          { label: "Llamada" },
        ]}
      />
      <h1 className="mb-1 max-w-3xl text-[26px] font-semibold leading-tight tracking-tight">{minutes.question}</h1>
      <p className="mono mb-8 text-xs text-muted-foreground">{id}</p>

      <ListenedButton product={pid} minutesId={id} heard={heard} />

      {segments.length === 0 ? (
        <Empty>
          Estas minutes no tienen call script. Para generar el audio de una reunión existente corré{" "}
          <span className="mono">agentsi render {id}</span>.
        </Empty>
      ) : (
        <CallPlayer
          segments={segments}
          participants={participants}
          audioSrc={hasAudio ? `/api/audio/${pid}/${id}` : null}
          renderCommand={`agentsi render ${id}`}
          timed={timed}
          product={pid}
          minutesId={id}
          heard={heard}
        />
      )}
    </div>
  );
}
