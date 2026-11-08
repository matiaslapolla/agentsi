import type { Metadata } from "next";
import { Arrow, Box, Diagram, Figure, Step, Zone } from "@/components/how/diagram";

export const metadata: Metadata = { title: "Cómo funciona" };

const RULES = [
  { title: "Los roles opinan. Vos decidís.", body: "Un rol propone prioridades y decisiones. Nada cambia en un producto sin tu aprobación." },
  { title: "AgentSI lee tu repo.", body: "El index y los roles solo leen. Una story escribe en su propia rama, y llega a main cuando vos la aceptás." },
  { title: "Todo queda en git.", body: "Cada minutes, story y decisión es un commit en el workspace. Podés auditar y revertir cada cambio." },
];

function Pieces() {
  const id = "pieces";
  return (
    <Figure
      title="1. Las piezas"
      caption={
        <>
          <p>Vos hablás con el PM en Claude Code. El PM corre el CLI <span className="mono">agentsi</span>, y el CLI hace el trabajo.</p>
          <p>Cada rol es una llamada a <span className="mono">claude -p</span> con su propio prompt. Los roles leen tu repo con Read, Grep y Glob.</p>
          <p>El CLI guarda las minutes en el workspace y el texto buscable en Postgres. La web app y el MCP leen esos dos lugares.</p>
        </>
      }
    >
      <Diagram id={id} w={850} h={445} label="Vos le pedís al PM en Claude Code. Claude Code corre el CLI. El CLI pregunta a los roles, indexa tu repo en Postgres y escribe el workspace.">
        <Box x={20} y={40} w={150} h={60} title="Vos" sub="pedís y decidís" tone="you" />
        <Box x={240} y={40} w={150} h={60} title="Claude Code" sub="/agentsi · PM" tone="pm" />
        <Box x={460} y={40} w={150} h={60} title="CLI agentsi" sub="meet · story · decide" tone="primary" />
        <Box x={680} y={40} w={150} h={60} title="Roles" sub="claude -p por rol" tone="scout" />

        <Box x={20} y={180} w={150} h={60} title="Web app" sub="127.0.0.1:3361" />
        <Box x={240} y={180} w={150} h={60} title="MCP agentsi" sub="search · get_doc" />
        <Box x={680} y={180} w={150} h={60} title="Tu repo" sub="solo lectura" tone="repo" />

        <Zone x={225} y={290} w={400} h={120} title="Datos de AgentSI" bottom />
        <Box x={240} y={320} w={150} h={60} title="Postgres" sub="knowledge base" />
        <Box x={460} y={320} w={150} h={60} title="Workspace" sub="repo git de datos" />

        <Arrow id={id} points={[[170, 70], [240, 70]]} label="pedís" flow />
        <Arrow id={id} points={[[390, 70], [460, 70]]} label="corre" flow />
        <Arrow id={id} points={[[610, 70], [680, 70]]} label="pregunta" flow />
        <Arrow id={id} points={[[755, 100], [755, 180]]} label="leen" at={[785, 144]} />
        <Arrow id={id} points={[[315, 100], [315, 180]]} label="busca" at={[345, 144]} />
        <Arrow id={id} points={[[95, 100], [95, 180]]} label="mirás" at={[127, 144]} />
        <Arrow id={id} points={[[315, 240], [315, 320]]} label="lee" at={[340, 270]} />
        <Arrow id={id} points={[[575, 100], [575, 320]]} label="commitea" at={[575, 200]} flow />
        <Arrow id={id} points={[[490, 100], [490, 270], [360, 270], [360, 320]]} label="indexa" at={[425, 262]} />
        <Arrow id={id} points={[[95, 240], [95, 350], [225, 350]]} label="lee" at={[160, 342]} />
        <Arrow id={id} points={[[755, 240], [755, 428], [425, 428], [425, 350], [390, 350]]} label="docs, git log, issues" at={[590, 421]} tone="repo" />
      </Diagram>
    </Figure>
  );
}

function Repo() {
  const id = "repo";
  return (
    <Figure
      title="2. Qué toca en tu repo"
      caption={
        <>
          <p>El index lee los docs, el journal, el git log de 90 días y las issues abiertas. Guarda el texto en Postgres, fuera de tu repo.</p>
          <p>Con Stories mode, una story session trabaja en <span className="mono">.worktrees/</span>, en la rama <span className="mono">story/&lt;id&gt;</span>. Un sandbox sin red la aísla.</p>
          <p>La rama llega a main solo cuando corrés <span className="mono">agentsi story accept</span>. Las minutes y la memoria nunca entran en tu repo.</p>
        </>
      }
    >
      <Diagram id={id} w={830} h={330} label="El index y los roles leen docs, journal, historia e issues de tu repo. Una story session escribe solo en su rama de .worktrees. La rama llega a main cuando vos aceptás.">
        <Zone x={250} y={14} w={320} h={300} title="Tu repo · miapp" tone="repo" />
        <Box x={280} y={44} w={260} h={40} title="Docs" start />
        <Box x={280} y={92} w={260} h={40} title="Journal" start />
        <Box x={280} y={140} w={260} h={40} title="Historia" start />
        <Box x={280} y={188} w={260} h={40} title="Issues de GitHub" dashed start />
        <text x={530} y={69} textAnchor="end" fontSize={10.5} fill="var(--muted-foreground)" className="mono">README · docs/**/*.md</text>
        <text x={530} y={117} textAnchor="end" fontSize={10.5} fill="var(--muted-foreground)" className="mono">.claude/JOURNAL.md</text>
        <text x={530} y={165} textAnchor="end" fontSize={10.5} fill="var(--muted-foreground)" className="mono">git log · 90 días</text>
        <text x={530} y={213} textAnchor="end" fontSize={10.5} fill="var(--muted-foreground)" className="mono">gh · abiertas</text>
        <Box x={280} y={252} w={110} h={46} title="main" tone="repo" />
        <Box x={430} y={252} w={110} h={46} title="story/<id>" sub=".worktrees/" tone="dev" />

        <Box x={20} y={106} w={170} h={60} title="Knowledge base" sub="Postgres" />
        <Box x={640} y={70} w={170} h={60} title="Roles" sub="Read · Grep · Glob" tone="scout" />
        <Box x={640} y={238} w={170} h={60} title="Story session" sub="bwrap · sin red" tone="dev" />

        {[64, 112, 160, 208].map((y) => (
          <Arrow key={y} id={id} points={[[280, y], [190, 136]]} />
        ))}
        <text x={222} y={96} textAnchor="middle" fontSize={11.5} fill="currentColor" stroke="var(--background)" strokeWidth={4} paintOrder="stroke">index lee</text>
        <Arrow id={id} points={[[640, 100], [540, 100]]} label="leen" />
        <Arrow id={id} points={[[640, 268], [540, 268]]} label="commitea" flow tone="dev" />
        <Arrow id={id} points={[[430, 275], [390, 275]]} label="accept" at={[410, 247]} tone="you" />
      </Diagram>
    </Figure>
  );
}

const ROLES = [
  { role: "dev", label: "Dev" },
  { role: "qa", label: "QA" },
  { role: "social", label: "Social" },
  { role: "scout", label: "Scout" },
];

function Meeting() {
  const id = "meeting";
  return (
    <Figure
      title="3. Una reunión"
      caption={
        <>
          <p>El CLI actualiza el index y arma un snapshot del producto. Cada rol recibe el snapshot y la pregunta, y responde una vez con fuentes.</p>
          <p>El PM ordena las prioridades y escribe las minutes. Un check de código valida cada fuente y la cita al north star. Si el check falla, el PM repara una vez.</p>
          <p>Las minutes quedan en el workspace con un commit. Después vos marcás las prioridades y decidís cada decisión.</p>
        </>
      }
    >
      <Diagram id={id} w={880} h={320} label="La pregunta va al snapshot. Cada rol responde en paralelo. El PM sintetiza. Un check valida. Las minutes se guardan y vos marcás y decidís.">
        <Box x={20} y={120} w={110} h={56} title="Pregunta" sub="agentsi meet" tone="you" />
        <Box x={160} y={120} w={120} h={56} title="Snapshot" sub="index + contexto" />
        {ROLES.map((r, i) => (
          <Box key={r.role} x={320} y={30 + i * 64} w={120} h={46} title={r.label} tone={r.role} />
        ))}
        <Box x={480} y={120} w={110} h={56} title="PM" sub="sintetiza" tone="pm" />
        <Box x={620} y={120} w={100} h={56} title="Check" sub="código" />
        <Box x={760} y={120} w={110} h={56} title="Minutes" sub="commit" tone="primary" />
        <Box x={750} y={240} w={126} h={56} title="Vos" sub="marcás · decidís" tone="you" />
        <Box x={600} y={240} w={120} h={56} title="Call" sub="--audio" dashed />

        <Step x={20} y={120} n={1} />
        <Step x={160} y={120} n={2} />
        <Step x={320} y={30} n={3} />
        <Step x={480} y={120} n={4} />
        <Step x={620} y={120} n={5} />
        <Step x={760} y={120} n={6} />

        <Arrow id={id} points={[[130, 148], [160, 148]]} />
        {ROLES.map((r, i) => (
          <Arrow key={r.role} id={id} points={[[280, 148], [320, 53 + i * 64]]} flow={i === 0} />
        ))}
        {ROLES.map((r, i) => (
          <Arrow key={r.role} id={id} points={[[440, 53 + i * 64], [480, 148]]} />
        ))}
        <Arrow id={id} points={[[590, 148], [620, 148]]} />
        <Arrow id={id} points={[[720, 148], [760, 148]]} flow />
        <Arrow id={id} points={[[670, 120], [670, 92], [535, 92], [535, 120]]} label="falla: repara 1 vez" at={[602, 84]} dashed tone="fail" />
        <Arrow id={id} points={[[815, 176], [815, 240]]} label="te toca" at={[846, 212]} tone="you" />
        <Arrow id={id} points={[[780, 176], [780, 210], [660, 210], [660, 240]]} label="opcional" at={[720, 202]} dashed />
      </Diagram>
    </Figure>
  );
}

function Story() {
  const id = "story";
  return (
    <Figure
      title="4. Una story, de la idea al merge"
      caption={
        <>
          <p>El PM escribe la story con 2 a 5 criterios. Cada criterio es una línea given, when, then con su archivo de test. Vos la aprobás o pedís cambios.</p>
          <p>QA escribe los tests primero, y cada test tiene que fallar. Después el rol dueño implementa. Tests y gate corren en un sandbox sin red.</p>
          <p>Si un check falla, el rol dueño tiene un intento más. Vos revisás el diff y aceptás o descartás.</p>
        </>
      }
    >
      <Diagram id={id} w={860} h={320} label="El PM propone una story. Vos la aprobás. Una rama en un worktree recibe los tests de QA y el código del rol dueño. Los checks corren en un sandbox. Vos aceptás o descartás.">
        <Box x={20} y={60} w={160} h={60} title="PM propone" sub="story new · suggest" tone="pm" />
        <Box x={230} y={60} w={160} h={60} title="Vos aprobás" sub="story approve" tone="you" />
        <Box x={440} y={60} w={160} h={60} title="Worktree" sub="rama story/<id>" />
        <Box x={650} y={60} w={190} h={60} title="QA escribe tests" sub="cada uno debe fallar" tone="qa" />

        <Box x={650} y={230} w={190} h={60} title="Rol dueño implementa" sub="Edit · Write" tone="dev" />
        <Box x={440} y={230} w={160} h={60} title="Checks" sub="tests + gate · bwrap" />
        <Box x={230} y={230} w={160} h={60} title="Lista" sub="story review" tone="primary" />
        <Box x={20} y={230} w={160} h={60} title="Vos resolvés" sub="accept · reject" tone="you" />

        <Step x={20} y={60} n={1} />
        <Step x={230} y={60} n={2} />
        <Step x={440} y={60} n={3} />
        <Step x={650} y={60} n={4} />
        <Step x={840} y={230} n={5} />
        <Step x={600} y={230} n={6} />
        <Step x={390} y={230} n={7} />
        <Step x={180} y={230} n={8} />

        <Arrow id={id} points={[[180, 90], [230, 90]]} />
        <Arrow id={id} points={[[390, 90], [440, 90]]} label="story run" at={[415, 50]} flow />
        <Arrow id={id} points={[[600, 90], [650, 90]]} />
        <Arrow id={id} points={[[790, 120], [790, 230]]} label="commit tests" at={[790, 160]} />
        <Arrow id={id} points={[[650, 268], [600, 268]]} flow />
        <Arrow id={id} points={[[440, 268], [390, 268]]} label="pasan" at={[415, 260]} />
        <Arrow id={id} points={[[230, 268], [180, 268]]} label="diff" at={[205, 260]} />
        <Arrow id={id} points={[[520, 230], [520, 190], [690, 190], [690, 230]]} label="falla: 1 intento más" at={[605, 182]} dashed tone="fail" />
        <Arrow id={id} points={[[310, 60], [310, 30], [100, 30], [100, 60]]} label="pedís cambios" at={[205, 22]} dashed tone="you" />
      </Diagram>
    </Figure>
  );
}

function Learning() {
  const id = "learning";
  return (
    <Figure
      title="5. Cómo aprenden los roles"
      caption={
        <>
          <p>Cada reunión deja un log: qué checks fallaron, qué fuentes inventó un rol y cuánto costó. Tus marcas dicen qué prioridades sirvieron.</p>
          <p><span className="mono">agentsi reflect</span> lee el log y las marcas. Suma y retira ítems de memoria de cada agente, y la próxima reunión los usa.</p>
          <p>Si un rol falla seguido, reflect propone un prompt nuevo. Un replay lo prueba sobre reuniones viejas. Pasa solo si ninguna métrica empeora, y espera tu decisión.</p>
        </>
      }
    >
      <Diagram id={id} w={850} h={290} label="Las reuniones y tus marcas alimentan reflect. Reflect cura la memoria, que usa la próxima reunión. Un prompt nuevo pasa por replay y espera tu decisión.">
        <Box x={20} y={60} w={160} h={56} title="Reunión" sub="minutes" tone="pm" />
        <Box x={235} y={60} w={160} h={56} title="Log + marcas" sub="meetings.jsonl" />
        <Box x={450} y={60} w={160} h={56} title="Reflect" sub="agentsi reflect" tone="primary" />
        <Box x={665} y={60} w={165} h={56} title="Memoria" sub="ítems por agente" tone="qa" />

        <Box x={450} y={210} w={160} h={56} title="Prompt nuevo" sub="overlay del rol" tone="scout" />
        <Box x={235} y={210} w={160} h={56} title="Replay" sub="reuniones viejas" />
        <Box x={20} y={210} w={160} h={56} title="Vos decidís" sub="agentsi decide" tone="you" />

        <Arrow id={id} points={[[180, 88], [235, 88]]} label="registra" at={[207, 80]} flow />
        <Arrow id={id} points={[[395, 88], [450, 88]]} label="lee" at={[422, 80]} flow />
        <Arrow id={id} points={[[610, 88], [665, 88]]} label="cura" at={[637, 80]} flow />
        <Arrow id={id} points={[[747, 60], [747, 28], [100, 28], [100, 60]]} label="la próxima reunión la usa" at={[424, 20]} tone="qa" />
        <Arrow id={id} points={[[530, 116], [530, 210]]} label="si un rol falla seguido" at={[600, 168]} dashed />
        <Arrow id={id} points={[[450, 238], [395, 238]]} />
        <Arrow id={id} points={[[235, 238], [180, 238]]} label="pasa" at={[207, 230]} />
        <Arrow id={id} points={[[100, 210], [100, 116]]} label="aprobás" at={[134, 168]} tone="you" />
      </Diagram>
    </Figure>
  );
}

export default function HowItWorksPage() {
  return (
    <div className="space-y-10">
      <header className="max-w-[720px] space-y-3">
        <h1 className="text-3xl font-semibold tracking-[-0.02em]">Cómo funciona</h1>
        <p className="text-[16px] text-muted-foreground">
          AgentSI es una agencia de roles para tus productos. Los roles leen tu repo, se reúnen y te dan prioridades y decisiones. Vos aprobás cada cambio.
        </p>
      </header>

      <ul className="grid gap-3 sm:grid-cols-3">
        {RULES.map((r) => (
          <li key={r.title} className="rounded-[18px] border border-border bg-card p-4">
            <p className="font-semibold">{r.title}</p>
            <p className="mt-1 text-[14px] text-muted-foreground">{r.body}</p>
          </li>
        ))}
      </ul>

      <Pieces />
      <Repo />
      <Meeting />
      <Story />
      <Learning />
    </div>
  );
}
