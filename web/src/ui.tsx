import { createContext, forwardRef, useContext, type ComponentProps } from 'react';
import { Button as CupertinoButton } from '@react-cupertino-ui/button';
import { TextField } from '@react-cupertino-ui/text-field';

export type Translate = (key: string, values?: Record<string, string | number>) => string;
export const I18n = createContext<Translate>((key) => key);
export const useText = () => useContext(I18n);

export const Button = forwardRef<HTMLButtonElement, ComponentProps<typeof CupertinoButton>>((props, ref) =>
  <CupertinoButton ref={ref} variant="default" {...props} />
);
Button.displayName = 'Button';

export { TextField };

export function Notice({ id, error }: { id: string; error?: string | null }) {
  return <p id={id} className="notice error" role="alert" hidden={!error}>{error}</p>;
}
