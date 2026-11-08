import { Badge } from "@/components/ui/badge";
import { APPROVAL_LABEL, SESSION_LABEL, STATUS_LABEL, type Story } from "@/lib/stories";

export function StoryBadges({ story, status = true }: { story: Story; status?: boolean }) {
  const st = STATUS_LABEL[story.status];
  const ap = APPROVAL_LABEL[story.approval];
  const se = story.session && story.status === "not_implemented" ? SESSION_LABEL[story.session.state] : null;
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      <Badge>{story.type === "task" ? "tarea" : "story"}</Badge>
      {status && <Badge tone={st.tone}>{st.label}</Badge>}
      {story.status === "not_implemented" && <Badge tone={ap.tone}>{ap.label}</Badge>}
      {se && <Badge tone={se.tone}>sesión: {se.label}</Badge>}
    </span>
  );
}
