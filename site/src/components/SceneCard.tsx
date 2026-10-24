import type { SceneSummary } from "@/lib/scenes";

/*
 * One scene from the library: its own thumbnail, its title, and the facts a
 * performer picks a scene by — tier, passes, controls, whether it is
 * scripted. The thumbnail is a real frame of the scene (guide section 7), and
 * it carries no decorative caption over it.
 */

const FACT = "text-label font-mono text-[10px] tracking-[0.12em] uppercase";

export default function SceneCard({
  scene,
  detailed = false,
}: {
  scene: SceneSummary;
  /** Adds the description and tags, for the library page. */
  detailed?: boolean;
}) {
  const facts = [
    scene.tier !== undefined ? `Tier ${scene.tier}` : undefined,
    scene.passes > 0 ? `${scene.passes + 1} passes` : "1 pass",
    `${scene.controls} control${scene.controls === 1 ? "" : "s"}`,
    scene.hasScript ? "script.js" : undefined,
  ].filter((fact): fact is string => fact !== undefined);

  return (
    <article className="bg-panel flex min-w-0 flex-col">
      <div className="bg-code-ground relative aspect-video overflow-hidden">
        {scene.thumbnail ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={scene.thumbnail}
            alt={`A frame of ${scene.title}`}
            width={640}
            height={360}
            loading="lazy"
            decoding="async"
            className="block h-full w-full object-cover"
          />
        ) : null}
      </div>
      <div className="flex flex-1 flex-col gap-2.5 p-5">
        <div className="flex items-baseline justify-between gap-3">
          <h3 className="text-text min-w-0 truncate text-[16px] leading-snug font-semibold tracking-[-0.01em]">
            {scene.title}
          </h3>
          <span className="text-label flex-none font-mono text-[10px]">
            {scene.slug.slice(0, 2)}
          </span>
        </div>
        <p className={FACT}>{facts.join(" · ")}</p>
        {detailed ? (
          <>
            <p className="text-secondary text-[13.5px] leading-[1.6] text-pretty">
              {scene.description}
            </p>
            {scene.credit ? (
              <p className="text-label text-[12px] leading-normal">
                {scene.credit}
                {scene.license ? ` · ${scene.license}` : ""}
              </p>
            ) : null}
            {scene.tags.length > 0 ? (
              <ul aria-label="Tags" className="mt-auto flex list-none flex-wrap gap-1.5 pt-2">
                {scene.tags.map((tag) => (
                  <li
                    key={tag}
                    className="border-hairline text-secondary border px-2 py-0.5 font-mono text-[10.5px]"
                  >
                    {tag}
                  </li>
                ))}
              </ul>
            ) : null}
          </>
        ) : null}
      </div>
    </article>
  );
}
