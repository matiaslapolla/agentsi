import { notFound } from "next/navigation";
import { Crumbs } from "@/components/bits";
import { AuditSection, BriefSection, ReflectionsSection, RosterSection } from "@/components/roles-sections";
import { findProduct } from "@/lib/registry";
import { auditFor, briefFor, reflectionsFor, rosterFor } from "@/lib/roles-data";

export const dynamic = "force-dynamic";

export default async function RolesPage({ params }: { params: Promise<{ product: string }> }) {
  const { product: pid } = await params;
  const product = findProduct(pid);
  if (!product) notFound();

  const brief = await briefFor(pid);
  const [roster, audit] = await Promise.all([rosterFor(pid, brief), auditFor(pid)]);
  const reflections = await reflectionsFor(pid);

  return (
    <div>
      <Crumbs items={[{ label: "Productos", href: "/" }, { label: product.name, href: `/p/${pid}` }, { label: "Roles" }]} />
      <div className="mb-10">
        <h1 className="text-3xl font-semibold tracking-[-0.02em]">Roles</h1>
        <p className="mt-1 max-w-2xl text-muted-foreground">
          Quién opina sobre {product.name}, qué objetivos persigue y qué aprendió de las reuniones.
        </p>
      </div>
      <RosterSection roster={roster} />
      <BriefSection brief={brief} />
      <AuditSection audit={audit} />
      <ReflectionsSection reflections={reflections} product={pid} />
    </div>
  );
}
