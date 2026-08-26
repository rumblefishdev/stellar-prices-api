import CheckRoundedIcon from '@mui/icons-material/CheckRounded';
import ErrorOutlineRoundedIcon from '@mui/icons-material/ErrorOutlineRounded';
import LockOutlinedIcon from '@mui/icons-material/LockOutlined';
import Box from '@mui/material/Box';
import Divider from '@mui/material/Divider';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { alpha } from '@mui/material/styles';
import type { ReactNode } from 'react';

import builtByWordmark from '../assets/built-by.png';
import rumblefishLockup from '../assets/rumblefish-lockup.png';
import { color, radius } from '../theme/tokens';
import { DiscordIcon } from './DiscordIcon';
import { cardBorder } from './primitives';

/**
 * The login card and the pieces the five sign-in screens are assembled from
 * (Figma frame `778:2499`).
 *
 * The design is one card in three bands — a centred header, a body that
 * changes per state, and a darker legal footer — with two kinds of callout
 * inside the body. Building those as parameters rather than as five separate
 * screens is what keeps the states honest: they differ by their message and
 * their action, and a reader can see that at a glance instead of diffing five
 * near-identical layouts.
 *
 * **Colours are measured off the exported render, not guessed.** The Figma seat
 * this was built against had no MCP calls left, so the frame arrived as a PNG;
 * every value below was sampled from it, and the ones that turned out to be
 * design-system tokens (#272727 `Surface/Gray/Main`, #1a1a1a `…/Main-alt`,
 * #535353 `Stroke/Default`, #004f3b `Accent/Emerald/900`) are referenced
 * through `tokens.ts` rather than repeated as literals. The two that are NOT in
 * the token set — the error callout's #460809 fill and #82181a edge — are
 * named here as what they are.
 */

/** Discord's brand blurple. Their colour, so it is not a theme token. */
export const DISCORD = '#5865f2';

/**
 * The error callout's fill and edge.
 *
 * The fill IS a design-system variable — `Red/950`, confirmed against node
 * `997:2210` on 2026-08-26 — so it comes through `tokens.ts` like every other
 * one. The edge is still a sample with no variable behind it.
 */
const ERROR_SURFACE = color.red[950];
const ERROR_EDGE = '#82181a';

/**
 * The card shell.
 *
 * `component="section"` with the heading inside it, so each state is a landmark
 * a screen reader can jump to rather than a `<div>` that happens to look like a
 * panel.
 */
/**
 * The card's width, and the width of the column it sits in.
 *
 * Exported because `LoginSection` places "Back to landing" ABOVE the card and
 * has to line up with its left edge (Adam, 2026-08-26). As a literal in two
 * files the link drifts off the card the first time this number changes.
 */
export const LOGIN_CARD_MAX_WIDTH = 464;

export function LoginCard({
  title,
  titleComponent = 'h3',
  subtitle,
  children,
  footer,
}: {
  title: ReactNode;
  /**
   * The heading LEVEL, separate from the visual size.
   *
   * `h1` on `/login`, where this card is the whole page and its title is the
   * page's subject. `h3` on the landing, where the hero owns the `h1` and the
   * status panel's hidden `h2` names the section. Getting this wrong does not
   * change a pixel and does change whether the document outline makes sense.
   */
  titleComponent?: 'h1' | 'h2' | 'h3';
  subtitle: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <Box
      // A stable hook for the closed-portal assertion, which has to name the
      // thing that must stay empty. It used to be the whole document, then the
      // portal section — but the section now carries the design's "Back to
      // landing" link, which is in-page navigation and not a control that could
      // start an OAuth flow. The card is where a control WOULD go, so the card
      // is what the test scopes to; that keeps the assertion exactly as strict
      // as it was rather than teaching it to ignore a link.
      data-testid="login-card"
      sx={{
        width: '100%',
        maxWidth: LOGIN_CARD_MAX_WIDTH,
        borderRadius: `${radius.lg}px`,
        border: cardBorder,
        backgroundColor: color.surface.gray,
        overflow: 'hidden',
      }}
    >
      <Stack spacing={1.5} sx={{ p: 4, textAlign: 'center' }}>
        <BuiltBy />
        {/* Sized by `variant`, levelled by `component` — the design's 40 px
            title at whatever depth the page it is on requires. */}
        <Typography
          variant="h3"
          component={titleComponent}
          color="text.primary"
        >
          {title}
        </Typography>
        <Typography variant="body1" sx={{ color: color.text.tertiary }}>
          {subtitle}
        </Typography>
      </Stack>

      <Divider sx={{ borderColor: alpha(color.stroke.default, 0.45) }} />

      <Stack spacing={2.5} sx={{ p: 4, alignItems: 'stretch' }}>
        {children}
      </Stack>

      {footer && (
        <>
          <Divider sx={{ borderColor: alpha(color.stroke.default, 0.45) }} />
          <Box
            sx={{
              backgroundColor: color.surface.grayAlt,
              px: 3,
              py: 2,
              textAlign: 'center',
            }}
          >
            {footer}
          </Box>
        </>
      )}
    </Box>
  );
}

/**
 * "Built by Rumble Fish", the card's top line.
 *
 * ⚠️ **The design's own two images since 2026-08-26 (Adam supplied them).**
 * This was set as TEXT in Clash Display, because the Figma seat had no MCP
 * calls left to export the assets and a broken `<img>` at the top of the
 * sign-in card is a worse first impression than the name in the heading face.
 * The export arrived, so the placeholder is gone: neither the wordmark's
 * weight nor the lockup's mark can be reproduced by a font, and it showed.
 *
 * Both are @2x PNGs (144 × 34 and 192 × 48) drawn at half size, so they are
 * sharp on the retina display a reviewer will open this on.
 *
 * One `role="img"` over the pair rather than two `alt`s: it is a single lockup
 * that happens to ship as two files, and "Built by, image, Rumblefish, image"
 * is what a screen reader would otherwise read out. The halves are `alt=""`
 * because the label on the wrapper is the name.
 */
function BuiltBy() {
  return (
    <Stack
      role="img"
      aria-label="Built by Rumble Fish"
      direction="row"
      spacing={1}
      justifyContent="center"
      alignItems="center"
    >
      <Box
        component="img"
        src={builtByWordmark}
        alt=""
        sx={{ width: 72, height: 'auto', display: 'block' }}
      />
      <Box
        component="img"
        src={rumblefishLockup}
        alt=""
        sx={{ width: 96, height: 'auto', display: 'block' }}
      />
    </Stack>
  );
}

/**
 * The boxed message inside a state's body — the design's two variants.
 *
 * `neutral` carries a fact the visitor can act on; `error` carries a failure.
 * They differ by colour AND by icon AND by wording, which is task 0193's rule
 * about "could not verify" versus "not a member": a refusal the visitor can fix
 * must never look like the same event as one they cannot.
 */
export function Callout({
  variant,
  icon,
  title,
  children,
}: {
  variant: 'neutral' | 'error' | 'discord';
  icon?: ReactNode;
  /**
   * Optional: the states this slice inherits from tasks 0183 and 0185 are one
   * sentence with no headline, and inventing a bold line above them would be
   * this slice deciding copy that belongs to those tasks.
   */
  title?: ReactNode;
  children?: ReactNode;
}) {
  const skin = {
    neutral: {
      surface: color.surface.background,
      edge: alpha(color.stroke.default, 0.45),
      tile: color.surface.gray,
      glyph: color.text.secondary,
      heading: color.text.primary,
      fallbackIcon: <LockOutlinedIcon sx={{ fontSize: 18 }} />,
    },
    error: {
      surface: ERROR_SURFACE,
      edge: ERROR_EDGE,
      tile: ERROR_EDGE,
      glyph: color.text.error,
      heading: color.text.error,
      fallbackIcon: <ErrorOutlineRoundedIcon sx={{ fontSize: 18 }} />,
    },
    discord: {
      surface: color.surface.background,
      edge: alpha(color.stroke.default, 0.45),
      tile: DISCORD,
      glyph: color.white,
      heading: color.text.primary,
      fallbackIcon: <DiscordIcon sx={{ fontSize: 18 }} />,
    },
  }[variant];

  return (
    <Stack
      direction="row"
      spacing={2}
      sx={{
        p: 2,
        borderRadius: `${radius.md}px`,
        border: `1px solid ${skin.edge}`,
        backgroundColor: skin.surface,
        textAlign: 'left',
      }}
    >
      <Box
        aria-hidden
        sx={{
          flexShrink: 0,
          width: 32,
          height: 32,
          borderRadius: '8px',
          display: 'grid',
          placeItems: 'center',
          backgroundColor: skin.tile,
          color: skin.glyph,
        }}
      >
        {icon ?? skin.fallbackIcon}
      </Box>
      <Stack spacing={0.5} sx={{ minWidth: 0 }}>
        {title && (
          <Typography
            variant="body1"
            sx={{ fontWeight: 700, color: skin.heading }}
          >
            {title}
          </Typography>
        )}
        {children && (
          // `component="div"`, because callers pass the `<p>` elements those
          // other tasks own — and a `<p>` inside a `<p>` is markup the browser
          // silently repairs by closing the outer one, which moves the text out
          // of the box that was meant to contain it.
          //
          // ⚠️ **`margin: 0` on those `<p>`s belongs here** (Adam,
          // 2026-08-26). Nothing in this app resets it: `CssBaseline` paints
          // the floor and the code face and leaves paragraph margins alone,
          // and the dashboard's chrome — the one place that DOES zero them —
          // is a different subtree entirely. So every callout on the login
          // card was drawing the user agent's `1em` above and below its
          // sentence, inside a box padded for none: the red OAuth error box
          // was the obvious one, but `discord` and `neutral` had it too.
          //
          // Consecutive paragraphs get their gap from a rule rather than from
          // the collapsed margins they used to inherit, so a two-`<p>` callout
          // still reads as two paragraphs.
          <Typography
            variant="body2"
            component="div"
            sx={{
              color: color.text.secondary,
              '& p': { margin: 0 },
              '& p + p': { marginTop: 1 },
            }}
          >
            {children}
          </Typography>
        )}
      </Stack>
    </Stack>
  );
}

/** The "What you get" rule with its label sitting in the gap. */
export function LabelledRule({ children }: { children: ReactNode }) {
  return (
    <Divider
      sx={{
        borderColor: alpha(color.stroke.default, 0.45),
        '&::before, &::after': {
          borderColor: alpha(color.stroke.default, 0.45),
        },
      }}
    >
      <Typography variant="body2" sx={{ color: color.text.tertiary }}>
        {children}
      </Typography>
    </Divider>
  );
}

/**
 * The four-item "What you get" list.
 *
 * A real `<ul>`. The rows are a list to anyone reading the page and there is no
 * reason for them not to be one to a screen reader; the tick tiles are
 * decoration and are hidden from it.
 */
export function Benefits({
  items,
}: {
  items: readonly { text: string; kind: 'check' | 'discord' }[];
}) {
  return (
    <Stack component="ul" spacing={1.5} sx={{ m: 0, p: 0, listStyle: 'none' }}>
      {items.map(({ text, kind }) => (
        <Stack
          component="li"
          key={text}
          direction="row"
          spacing={1.5}
          alignItems="center"
        >
          <Box
            aria-hidden
            sx={{
              flexShrink: 0,
              width: 24,
              height: 24,
              borderRadius: '6px',
              display: 'grid',
              placeItems: 'center',
              backgroundColor:
                kind === 'check' ? color.accent.emerald[900] : DISCORD,
              color: kind === 'check' ? color.text.success : color.white,
            }}
          >
            {kind === 'check' ? (
              <CheckRoundedIcon sx={{ fontSize: 16 }} />
            ) : (
              <DiscordIcon sx={{ fontSize: 14 }} />
            )}
          </Box>
          <Typography variant="body2" sx={{ color: color.text.primary }}>
            {text}
          </Typography>
        </Stack>
      ))}
    </Stack>
  );
}
