import Link from "next/link";
import { notFound } from "next/navigation";
import { History, ListChecks, Workflow } from "lucide-react";
import { storyActions } from "@agentsi/core/story";
import { Crumbs, Empty, GlyphHeading } from "@/components/bits";
import { CliHint } from "@/components/cli-hint";
import { RoleAvatar } from "@/components/roster";
import { SourceChip } from "@/components/source-chip";
import { CriterionCard } from "@/components/stories/criterion-card";
import { SessionTimeline } from "@/components/stories/session-timeline";
import { StoryActions } from "@/components/stories/story-actions";
import { StoryBadges } from "@/components/stories/story-badges";
import { Badge } from "@/components/ui/badge";
import { findProduct } from "@/lib/registry";
import { roleMeta } from "@/lib/roles";
import { changesCommand, readStory, RESULT_LABEL, shortDate, storySentence, type Story } from "@/lib/stories";

export const dynamic = "force-dynamic";

const WEB_ACTIONS = ["approve", "changes", "discard"] as const;

/** The CLI step that follows the current state, when the web has no button for it. */
function nextCommand(s: Story): { text: string; command: string } | null {
  if (s.status !== "not_implemented") return null;
  if (s.approval === "changes_requested") return null;
  if (s.session?.state === "ready") return { text: "La sesión terminó. Revisá el diff y mergeá o descartá:", command: `agentsi story review ${s.id}` };
  if (s.session?.state === "failed") return { text: "La sesión falló. Descartá su rama para correrla otra vez:", command: `agentsi story reject ${s.id}` };
  if (s.session) return { text: "La sesión está en curso. Si se trabó, descartala:", command: `agentsi story reject ${s.id}` };
  if (s.approval === "approved" && !s.session) return { text: "Corré la sesión en Claude Code:", command: `agentsi story run ${s.id}` };
  return null;
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <dt className="label text-muted-foreground">{label}</dt>
      <dd className="mt-1 text-sm">{children}</dd>
    </div>
  );
}

export default async function StoryPage({ params }: { params: Promise<{ product: string; id: string }> }) {
  const { product: pid, id } = await params;
  const product = findProduct(pid);
  const story = product?.stories ? await readStory(pid, id) : null;
  if (!product || !story) notFound();

  const actions = storyActions(story).filter((a) => (WEB_ACTIONS as readonly string[]).includes(a));
  const next = nextCommand(story);
  const owner = roleMeta(story.owner_role);
  const results = [...story.results].reverse();

  return (
    <div>
      <Crumbs
        items={[
          { label: "Productos", href: "/" },
          { label: product.name, href: `/p/${pid}` },
          { label: "Stories", href: `/p/${pid}/stories` },
          { label: story.id },
        ]}
      />

      <header className="mb-6">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          <span className="mono text-xs text-muted-foreground">{story.id}</span>
          <StoryBadges story={story} />
        </div>
        <h1 className="mt-3 max-w-3xl text-balance text-[28px] font-semibold leading-tight tracking-[-0.02em]">{story.title}</h1>
      </header>

      <section aria-label="La story" className="mb-10 rounded-paper bg-card px-6 py-7 shadow-soft sm:px-9 sm:py-9">
        <p className="max-w-[46rem] text-pretty text-[21px] leading-[1.55] tracking-[-0.01em] sm:text-[24px]">
          {storySentence(story).map((part, i, all) => (
            <span key={part.lead}>
              <span className="text-muted-foreground">{part.lead} </span>
              <span className="font-medium">{part.text}</span>
              {i < all.length - 1 ? ", " : "."}
              {i < all.length - 1 && <br className="hidden sm:block" />}
            </span>
          ))}
        </p>
        <div className="mt-6 flex items-center gap-3 border-t border-border pt-5">
          <RoleAvatar role={story.owner_role} size={36} />
          <span>
            <span className="label block text-muted-foreground">La implementa</span>
            <span className="font-semibold">{owner.label}</span>
          </span>
        </div>
      </section>

      <div className="grid gap-x-10 gap-y-12 lg:grid-cols-[minmax(0,1fr)_300px]">
        <div className="min-w-0 space-y-12">
          <section>
            <GlyphHeading Icon={ListChecks} count={story.criteria.length}>
              Criterios de aceptación
            </GlyphHeading>
            <ol className="flex flex-col gap-3">
              {story.criteria.map((c, i) => (
                <CriterionCard key={i} criterion={c} n={i + 1} />
              ))}
            </ol>
          </section>

          <section>
            <GlyphHeading Icon={Workflow}>Sesión</GlyphHeading>
            {story.session ? (
              <SessionTimeline session={story.session} />
            ) : (
              <Empty>Todavía no corrió ninguna sesión.</Empty>
            )}
          </section>

          <section>
            <GlyphHeading Icon={History} count={results.length}>
              Resultados
            </GlyphHeading>
            {results.length === 0 ? (
              <Empty>Sin resultados: ninguna sesión llegó a la revisión.</Empty>
            ) : (
              <ol className="relative ml-1.5 space-y-5 border-l border-border pl-6">
                {results.map((r, i) => (
                  <li key={i} className="relative">
                    <span
                      className={`absolute -left-[31px] top-1.5 size-2.5 rounded-full ring-4 ring-background ${r.result === "discarded" ? "bg-status-neglected" : "bg-status-fresh"}`}
                      aria-hidden
                    />
                    <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                      <Badge tone={RESULT_LABEL[r.result].tone}>{RESULT_LABEL[r.result].label}</Badge>
                      <span className="text-sm text-muted-foreground">{shortDate(r.at)}</span>
                      {r.commit && <span className="mono text-xs text-muted-foreground">{r.commit.slice(0, 7)}</span>}
                    </div>
                  </li>
                ))}
              </ol>
            )}
          </section>
        </div>

        <aside className="order-first space-y-4 lg:sticky lg:top-6 lg:order-none lg:self-start">
          {story.approval === "changes_requested" && story.status === "not_implemented" && (
            <div className="rounded-lg bg-status-waiting/10 px-5 py-4">
              <p className="font-medium text-status-waiting">Pediste cambios</p>
              <p className="mt-1 text-sm">{story.changes_note}</p>
              <p className="mt-3 text-sm text-muted-foreground">El PM reescribe la story cuando corrés esto en Claude Code:</p>
              <div className="mt-2">
                <CliHint>{changesCommand(story.id, story.changes_note ?? "")}</CliHint>
              </div>
            </div>
          )}
          {actions.length > 0 && <StoryActions product={pid} id={story.id} approval={story.approval} actions={actions} />}
          {next && (
            <div className="rounded-lg bg-card px-5 py-4 shadow-soft">
              <p className="text-sm text-muted-foreground">{next.text}</p>
              <div className="mt-2">
                <CliHint>{next.command}</CliHint>
              </div>
            </div>
          )}

          <dl className="space-y-4 rounded-lg bg-card px-5 py-4 shadow-soft">
            {story.objective && (
              <Fact label="Objetivo del brief">
                <Badge tone="accent">{story.objective}</Badge>
              </Fact>
            )}
            <Fact label="La propuso">{story.proposed_by === "pm" ? "El PM" : "Vos"}</Fact>
            {story.minutes && (
              <Fact label="En la reunión">
                <Link href={`/p/${pid}/m/${story.minutes}`} className="mono break-all text-xs text-primary underline-offset-4 hover:underline">
                  {story.minutes}
                </Link>
              </Fact>
            )}
            {story.sources.length > 0 && (
              <Fact label="Fuentes">
                <span className="flex flex-wrap gap-1.5">
                  {story.sources.map((s, i) => (
                    <SourceChip key={i} source={s} product={pid} />
                  ))}
                </span>
              </Fact>
            )}
            <Fact label="Creada">{shortDate(story.created_at)}</Fact>
            <Fact label="Actualizada">{shortDate(story.updated_at)}</Fact>
          </dl>
        </aside>
      </div>
    </div>
  );
}
