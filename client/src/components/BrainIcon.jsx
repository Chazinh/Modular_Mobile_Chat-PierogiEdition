import React from 'react';

export default function BrainIcon({ size = 24, strokeWidth = 1.8, ...props }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    >
      <path d="M12 4.5C11.7 2.7 9.5 2 8.1 3.2C6.6 3 5.3 4 5.1 5.5C3.3 5.7 2.5 7.4 3.1 9C1.9 10.6 2.3 12.8 3.9 13.7C3.5 15.6 4.9 17.1 6.7 17.1C7.8 19 10.4 19.4 12 17.7C13.6 19.4 16.2 19 17.3 17.1C19.1 17.1 20.5 15.6 20.1 13.7C21.7 12.8 22.1 10.6 20.9 9C21.5 7.4 20.7 5.7 18.9 5.5C18.7 4 17.4 3 15.9 3.2C14.5 2 12.3 2.7 12 4.5Z" />
      <path d="M12 4.5v13.2M10.5 18.7V21h3v-2.3" />
      <path d="M5.1 5.5c.1 1.6 1.3 2.4 2.7 2.2M3.1 9c1.3-.5 2.7 0 3.2 1.2M3.9 13.7c1.2-1 2.7-.9 3.6.1M6.7 17.1c1.1-.4 1.6-1.5 1.4-2.5M8.4 10.3c1.4.1 2.1 1.1 1.8 2.3" />
      <path d="M18.9 5.5c-.1 1.6-1.3 2.4-2.7 2.2M20.9 9c-1.3-.5-2.7 0-3.2 1.2M20.1 13.7c-1.2-1-2.7-.9-3.6.1M17.3 17.1c-1.1-.4-1.6-1.5-1.4-2.5M15.6 10.3c-1.4.1-2.1 1.1-1.8 2.3" />
    </svg>
  );
}
