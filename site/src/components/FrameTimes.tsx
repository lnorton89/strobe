import { FRAME_BUDGET_MS, type RenderScaleSweep } from "@/lib/benchmarks";

/*
 * The library's frame times on one machine, as a table with a bar under each
 * scene's name. A real table, so the numbers read without the bars; the bars
 * show the budget at a glance, and one that runs past twice the budget stops at
 * the edge, with the number still there.
 */

/** The bar's full width, in multiples of the budget. */
const SCALE = 2;

export default function FrameTimes({
  sweep,
  titles,
}: {
  sweep: RenderScaleSweep;
  /** The scenes' display names, by slug. */
  titles: ReadonlyMap<string, string>;
}) {
  const scenes = [...sweep.scenes].sort((a, b) => a.full - b.full);
  return (
    <div className="border-hairline overflow-x-auto border" tabIndex={0} role="region" aria-label="Frame times on a ThinkPad T480">
      <table className="w-full border-collapse text-left">
        <thead className="bg-panel">
          <tr>
            <th scope="col" className="text-label px-4 py-2.5 font-mono text-[10px] font-medium tracking-[0.14em] uppercase">
              Scene
            </th>
            <th scope="col" className="text-label px-3 py-2.5 text-right font-mono text-[10px] font-medium tracking-[0.14em] uppercase">
              720p
            </th>
            <th scope="col" className="text-label px-4 py-2.5 text-right font-mono text-[10px] font-medium tracking-[0.14em] uppercase">
              360p
            </th>
          </tr>
        </thead>
        <tbody>
          {scenes.map((scene) => {
            const over = scene.full > FRAME_BUDGET_MS;
            return (
              <tr key={scene.slug} className="border-hairline border-t">
                <th scope="row" className="min-w-40 px-4 py-2 font-normal">
                  <span className="text-bright block text-[13px] leading-snug">
                    {titles.get(scene.slug) ?? scene.slug}
                  </span>
                  <span aria-hidden="true" className="bg-panel relative mt-1.5 block h-1">
                    <span
                      className={`absolute inset-y-0 left-0 ${over ? "bg-magenta" : "bg-accent"}`}
                      style={{
                        width: `${Math.min(scene.full / (FRAME_BUDGET_MS * SCALE), 1) * 100}%`,
                      }}
                    />
                    {/* The budget, half way along: the guide's fine calibration tick. */}
                    <span className="bg-text absolute -inset-y-px left-1/2 w-px" />
                  </span>
                </th>
                <td className={`px-3 py-2 text-right font-mono text-[13px] tabular-nums ${over ? "text-text" : "text-secondary"}`}>
                  {scene.full.toFixed(1)}
                </td>
                <td className={`px-4 py-2 text-right font-mono text-[13px] tabular-nums ${scene.half > FRAME_BUDGET_MS ? "text-text" : "text-secondary"}`}>
                  {scene.half.toFixed(1)}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
