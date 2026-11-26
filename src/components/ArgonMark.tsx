import React, { useId } from 'react';

interface ArgonMarkProps {
  className?: string;
  size?: number;
  monochrome?: boolean;
  withTile?: boolean;
}

/**
 * Obsidian Crimson Prism emblem for ARGON.
 * Dual architectural "A" prism blades framing a platinum-rose diamond core
 * and a velvet burgundy download vector.
 */
export const ArgonMark: React.FC<ArgonMarkProps> = ({
  className,
  size = 24,
  monochrome = false,
  withTile = false
}) => {
  const uid = useId();
  const bgId = `argon-bg-${uid}`;
  const haloId = `argon-halo-${uid}`;
  const strokeId = `argon-stroke-${uid}`;
  const crimsonId = `argon-crimson-${uid}`;

  if (withTile) {
    return (
      <svg
        width={size}
        height={size}
        viewBox="0 0 64 64"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        className={className}
        aria-hidden="true"
      >
        <defs>
          <linearGradient id={bgId} x1="8" y1="4" x2="56" y2="60" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#11050A" />
            <stop offset="100%" stopColor="#070204" />
          </linearGradient>
          <radialGradient id={haloId} cx="50%" cy="46%" r="54%">
            <stop offset="0%" stopColor="#9E1B42" stopOpacity="0.32" />
            <stop offset="55%" stopColor="#72142B" stopOpacity="0.14" />
            <stop offset="100%" stopColor="#070204" stopOpacity="0" />
          </radialGradient>
          <linearGradient id={strokeId} x1="0" y1="0" x2="64" y2="64" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#E02447" stopOpacity="0.45" />
            <stop offset="50%" stopColor="#FFFFFF" stopOpacity="0.14" />
            <stop offset="100%" stopColor="#FFFFFF" stopOpacity="0.06" />
          </linearGradient>
          <linearGradient id={crimsonId} x1="32" y1="37" x2="32" y2="51" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#E02447" />
            <stop offset="55%" stopColor="#9E1B42" />
            <stop offset="100%" stopColor="#72142B" />
          </linearGradient>
        </defs>
        <rect width="64" height="64" rx="16" fill={monochrome ? 'var(--argon-surface, #14151B)' : `url(#${bgId})`} />
        {!monochrome && <rect width="64" height="64" rx="16" fill={`url(#${haloId})`} />}
        <rect
          x="0.75"
          y="0.75"
          width="62.5"
          height="62.5"
          rx="15.25"
          stroke={monochrome ? 'var(--argon-border, rgba(255,255,255,0.12))' : `url(#${strokeId})`}
          strokeWidth="1.25"
        />
        <path d="M27.5 13H36.5L52 50H43L32 23.5L21 50H12L27.5 13Z" fill="currentColor" />
        <path d="M32 30.5L35 33.5L32 36.5L29 33.5Z" fill={monochrome ? 'currentColor' : '#FFFFFF'} />
        {!monochrome && <path d="M32 31.8L33.7 33.5L32 35.2L30.3 33.5Z" fill="#FFB3C1" />}
        <path
          d="M29.9 38.2H34.1V43.2H37.8L32 51L26.2 43.2H29.9V38.2Z"
          fill={monochrome ? 'currentColor' : `url(#${crimsonId})`}
        />
      </svg>
    );
  }

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={crimsonId} x1="32" y1="37" x2="32" y2="52" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#E02447" />
          <stop offset="55%" stopColor="#9E1B42" />
          <stop offset="100%" stopColor="#72142B" />
        </linearGradient>
      </defs>
      <path d="M27.5 12H36.5L52.5 51H43L32 23.5L21 51H11.5L27.5 12Z" fill="currentColor" />
      <path d="M32 30L35.2 33.2L32 36.4L28.8 33.2Z" fill={monochrome ? 'currentColor' : 'var(--argon-bright, #FFFFFF)'} />
      {!monochrome && <path d="M32 31.4L33.8 33.2L32 35L30.2 33.2Z" fill="#E02447" />}
      <path
        d="M29.8 38H34.2V43.2H38.2L32 52L25.8 43.2H29.8V38Z"
        fill={monochrome ? 'currentColor' : `url(#${crimsonId})`}
      />
    </svg>
  );
};
