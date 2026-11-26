import { AnimatePresence, motion } from 'motion/react';
import { Button } from 'krts.ui';
import type { ModelStatus as Status } from '../lib/types';
import { api } from '../lib/ipc';

const mb = (n: number) => (n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.round(n / 1e6)} MB`);

/**
 * Download state under a control. Appears only while something is
 * happening; a finished download says "Ready" briefly and leaves.
 */
export function ModelStatus({ status }: { status: Status | undefined }) {
  const show = status && status.state !== 'ready' && status.state !== 'missing';
  return (
    <AnimatePresence initial={false}>
      {show && (
        <motion.div
          key="status"
          initial={{ opacity: 0, height: 0 }}
          animate={{ opacity: 1, height: 'auto' }}
          exit={{ opacity: 0, height: 0 }}
          transition={{ duration: 0.22, ease: [0.23, 1, 0.32, 1] }}
          className="overflow-hidden"
        >
          <div className="pt-3">
            {status.state === 'downloading' && <Downloading received={status.received} total={status.total} />}
            {status.state === 'error' && (
              <div className="flex items-center justify-between gap-3 text-[12.5px]">
                <span className="text-danger">{status.message}</span>
                <Button size="xs" variant="secondary" onPress={() => api.retryDownload()}>
                  Retry
                </Button>
              </div>
            )}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function Downloading({ received, total }: { received: number; total: number }) {
  const pct = total > 0 ? Math.min(100, (received / total) * 100) : 0;
  return (
    <div className="flex items-center gap-3">
      <div className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-surface">
        <div
          className="absolute inset-y-0 left-0 w-full origin-left rounded-full bg-primary transition-transform duration-300 ease-linear"
          style={{ transform: `scaleX(${pct / 100})` }}
        />
      </div>
      <span className="w-[7.5rem] text-right text-[12px] tabular-nums text-fg-muted">
        {mb(received)} / {mb(total)}
      </span>
    </div>
  );
}
