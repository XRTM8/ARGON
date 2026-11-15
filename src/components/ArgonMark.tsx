import React from 'react';

interface ArgonMarkProps {
  className?: string;
  size?: number;
  monochrome?: boolean;
  withTile?: boolean;
}

/**
 * Corporate geometric emblem for ARGON.
 * Dual architectural "A" prism blades converging around a precision cobalt download vector.
 */
export const ArgonMark: React.FC<ArgonMarkProps> = ({
  className,
  size = 24,
  monochrome = false,
  withTile = false
}) => {
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
          <linearGradient id="argon-mark-bg" x1="8" y1="4" x2="56" y2="60" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#161D2A" />
            <stop offset="100%" stopColor="#090C11" />
          </linearGradient>
          <linearGradient id="argon-mark-cobalt" x1="32" y1="33" x2="32" y2="49" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#60A5FA" />
            <stop offset="100%" stopColor="#2563EB" />
          </linearGradient>
        </defs>
        <rect width="64" height="64" rx="16" fill="url(#argon-mark-bg)" />
        <rect x="0.75" y="0.75" width="62.5" height="62.5" rx="15.25" stroke="#283448" strokeWidth="1.5" />
        <path d="M29 14.5H35L50.5 49H42.3L32 25.4L21.7 49H13.5L29 14.5Z" fill="#F8FAFC" />
        <path d="M29.9 33.5H34.1V40.6H37.3L32 49L26.7 40.6H29.9V33.5Z" fill={monochrome ? '#F8FAFC' : 'url(#argon-mark-cobalt)'} />
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
      <path
        d="M28.5 11H35.5L53 51H43.6L32 24.4L20.4 51H11L28.5 11Z"
        fill="currentColor"
      />
      <path
        d="M29.6 33.2H34.4V41.2H38.2L32 51L25.8 41.2H29.6V33.2Z"
        fill={monochrome ? 'currentColor' : 'var(--argon-accent, #3B82F6)'}
      />
    </svg>
  );
};
