import Link from "next/link";
import StrobeMark from "@/components/brand/StrobeMark";
import { NAV_ICONS } from "@/components/brand/icons";
import { NAV } from "@/lib/site";

export const metadata = {
  title: "Not found",
};

export default function NotFound() {
  return (
    <div className="mx-auto flex max-w-[1360px] flex-col px-6 py-32 sm:px-10">
      <StrobeMark size={64} variant="mono" color="var(--hairline-strong)" />
      <p className="text-accent-text mt-10 font-mono text-[11px] tracking-[0.2em] uppercase">
        404
      </p>
      <h1 className="text-text mt-6 max-w-[18ch] text-[40px] leading-[1.05] font-semibold tracking-[-0.03em] sm:text-[56px]">
        No scene loaded at this address.
      </h1>
      <p className="text-secondary mt-6 max-w-[56ch] text-[18px] leading-[1.55]">
        The page you asked for does not exist. One of these will.
      </p>

      <nav aria-label="Site sections" className="rule-grid mt-14 grid w-fit sm:grid-cols-2">
        {NAV.map((item) => {
          const Icon = NAV_ICONS[item.icon];
          return (
            <Link
              key={item.href}
              href={item.href}
              className="bg-panel hover:bg-panel-raised panel-hover flex min-w-[240px] flex-col gap-1.5 px-6 py-5"
            >
              <span className="text-text flex items-center gap-2.5 text-[13px] font-semibold tracking-widest uppercase">
                <Icon size={16} />
                {item.label}
              </span>
              <span className="text-label pl-[26px] font-mono text-[11px]">
                {item.blurb}
              </span>
            </Link>
          );
        })}
      </nav>
    </div>
  );
}
