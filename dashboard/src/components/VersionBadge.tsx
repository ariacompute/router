import { useEffect, useState } from 'react';
import { getJson, type BuildVersion } from '../api';

/// Self-contained build-version badge (v{version}@{commit}), mirroring the
/// sidebar footer on the main dashboard. Fetches /v1/router/version itself so
/// it can be dropped into any page (e.g. the login / register dialogs) without
/// threading the build value down through props.
export default function VersionBadge({ className }: { className?: string }) {
  const [build, setBuild] = useState<BuildVersion | null>(null);

  useEffect(() => {
    let cancelled = false;
    getJson<BuildVersion>('/v1/router/version')
      .then((v) => {
        if (!cancelled) setBuild(v);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  if (!build) return null;

  return (
    <span
      className={className}
      title={`Aria Router v${build.version} @ ${build.commit}`}
      style={{
        fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
        fontSize: '0.7rem',
        color: 'var(--text-soft)',
        letterSpacing: '0.01em',
        userSelect: 'none',
      }}
    >
      v{build.version}@{build.commit}
    </span>
  );
}
