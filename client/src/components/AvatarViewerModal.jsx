import { tr, useLanguage } from '../i18n.js';
import React from 'react';
import { X } from 'lucide-react';
import { useLayer } from '../nav.js';
import '../drawer.css';

export default function AvatarViewerModal({ item, onClose }) {
  useLanguage();
  useLayer(!!item, onClose);

  if (!item) return null;

  const { name, avatar } = item;
  const hasImage = avatar && (avatar.startsWith('http') || avatar.startsWith('data:image/'));

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-md mc-modal-backdrop"
      onClick={onClose}
    >
      <div
        className="mc-glass-pill relative flex flex-col items-center overflow-hidden rounded-[30px] p-4 shadow-2xl max-w-[90vw] max-h-[82vh] mc-modal-card"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex w-full items-center justify-between pb-3 px-1 gap-3">
          <span className="truncate text-base font-semibold text-white">{name || tr("Avatar")}</span>
          <button
            onClick={onClose}
            className="flex h-8 w-8 items-center justify-center rounded-full bg-white/10 text-white hover:bg-white/20 active:scale-90 transition-transform"
            aria-label={tr("Fechar")}
          >
            <X size={18} />
          </button>
        </div>

        <div className="flex items-center justify-center overflow-hidden rounded-2xl bg-black/30 border border-white/10">
          {hasImage ? (
            <img
              src={avatar}
              alt={name || ''}
              className="max-h-[68vh] max-w-[82vw] object-contain select-none"
            />
          ) : (
            <div className="flex h-52 w-52 items-center justify-center text-5xl font-bold text-white/80">
              {[...(name || '?').trim()][0]?.toUpperCase() || '?'}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
