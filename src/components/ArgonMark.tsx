import React from 'react';

interface ArgonMarkProps {
  className?: string;
  size?: number;
  monochrome?: boolean;
}

/**
 * Corporate geometric emblem for ARGON.
 * Dual architectural "A" prism blades converging around a precision cobalt download vector.
 */
export const ArgonMark: React.FC<ArgonMarkProps> = ({
  className,
  size = 24,
  monochrome = false
}) => {
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
