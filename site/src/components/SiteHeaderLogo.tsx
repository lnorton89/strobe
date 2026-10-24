import Link from "next/link";
import StrobeMark from "@/components/brand/StrobeMark";
import Wordmark from "@/components/brand/Wordmark";

/*
 * The header's home link: the ribbon beside the wordmark, the guide's
 * horizontal lockup without its descriptor (a 64px header has no room for a
 * third line). At 28px the ribbon is drawn in its simplified small-size form,
 * as the guide asks below 32px. Below 400px there is room for the mark alone;
 * the link's aria-label names the destination either way.
 */
export default function SiteHeaderLogo() {
  return (
    <Link
      href="/"
      aria-label="Strobe — home"
      className="flex items-center gap-3 focus-visible:outline-offset-4"
    >
      <StrobeMark size={28} />
      <Wordmark size={18} className="hidden min-[400px]:inline-flex" />
    </Link>
  );
}
