import Button from '@mui/material/Button';
import Stack from '@mui/material/Stack';
import Typography from '@mui/material/Typography';
import { Link as RouterLink } from 'react-router-dom';

import { color } from '../theme/tokens';
import { LOGIN_ROUTE, SWAGGER_UI } from './links';
import { ArrowBadge, GradientSection } from './primitives';

/**
 * "Start building today" — the closing call to action.
 *
 * The same `canOfferKey` rule the hero and the navbar follow: while the portal
 * is shut there is nothing behind "Get API Key", so the page does not offer it.
 * What is left is the documentation button, promoted to the filled style —
 * a closing section with one greyed-out control reads as a page that failed to
 * load rather than a product that is not open yet.
 */
export function FinalCta({ canOfferKey }: { canOfferKey: boolean }) {
  return (
    <GradientSection from="alt" to="base" sx={{ textAlign: 'center' }}>
      <Stack
        spacing={2.5}
        alignItems="center"
        sx={{ maxWidth: 720, mx: 'auto' }}
      >
        <Typography variant="h2" component="h2" color="text.primary">
          Start building today
        </Typography>
        <Typography variant="body1" sx={{ color: color.text.secondary }}>
          Get instant access to the Prices API. Sign in with Discord, receive
          your key, and make your first call in under a minute.
        </Typography>
        <Stack
          direction={{ xs: 'column', sm: 'row' }}
          spacing={2}
          sx={{ pt: 1 }}
        >
          {canOfferKey && (
            <Button
              variant="contained"
              color="primary"
              component={RouterLink}
              to={LOGIN_ROUTE}
              endIcon={<ArrowBadge variant="onPrimary" />}
            >
              Get API Key
            </Button>
          )}
          {/* White, not the brand yellow: the design's second button here is
              light-on-dark, which is what keeps two filled buttons side by side
              from competing. */}
          <Button
            variant="contained"
            href={SWAGGER_UI}
            endIcon={<ArrowBadge variant="onPrimary" />}
            sx={{
              backgroundColor: color.white,
              color: color.black,
              '&:hover': { backgroundColor: color.gray[50] },
            }}
          >
            Read documentation
          </Button>
        </Stack>
      </Stack>
    </GradientSection>
  );
}
