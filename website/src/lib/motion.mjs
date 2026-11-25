/**
 * Scroll motion for the site's own pages and the docs.
 *
 * Elements marked `data-reveal` start slightly lowered and faded, and settle
 * when they come into view; `--i` staggers siblings. The class that hides them
 * is set by this script before the first paint, so without scripts, or with
 * reduced motion, everything is simply visible. The header gets a stronger
 * glass once the page has scrolled.
 */
export const motionScript = `(() => {
  const root = document.documentElement
  const scrolled = () => root.classList.toggle('scrolled', scrollY > 8)
  addEventListener('scroll', scrolled, { passive: true })
  document.addEventListener('DOMContentLoaded', scrolled)
  if (!('IntersectionObserver' in window) || matchMedia('(prefers-reduced-motion: reduce)').matches) return
  root.classList.add('motion')
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue
      entry.target.classList.add('shown')
      observer.unobserve(entry.target)
    }
  }, { rootMargin: '0px 0px -8% 0px' })
  // No threshold: a panel taller than the screen could never reach a share of itself.
  const watch = () => document.querySelectorAll('[data-reveal]:not(.shown)').forEach((element) => observer.observe(element))
  document.addEventListener('DOMContentLoaded', watch)
})()`
