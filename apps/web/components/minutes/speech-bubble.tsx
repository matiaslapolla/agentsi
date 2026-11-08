import { RoleAvatar } from "@/components/roster";
import { cn } from "@/lib/cn";
import { roleMeta } from "@/lib/roles";

/** A role's words in a bubble, with the role named under the tail. `tone` matches the fill to the section behind it. */
export function SpeechBubble({
  role,
  tone = "paper",
  children,
}: {
  role: string;
  tone?: "paper" | "white";
  children: React.ReactNode;
}) {
  return (
    <figure className="flex min-w-0 flex-col">
      <blockquote
        className={cn(
          "relative rounded-paper px-5 py-4 text-[15px] leading-relaxed text-foreground",
          "after:absolute after:-bottom-4 after:left-8 after:h-[18px] after:w-7 after:content-[''] after:[clip-path:polygon(0_0,100%_0,0_100%)]",
          tone === "paper" ? "bg-background after:bg-background" : "bg-card shadow-soft after:bg-card",
        )}
      >
        {children}
      </blockquote>
      <figcaption className="ml-6 mt-6 flex items-center gap-2">
        <RoleAvatar role={role} size={26} />
        <span className="text-sm font-semibold">{roleMeta(role).label}</span>
      </figcaption>
    </figure>
  );
}
