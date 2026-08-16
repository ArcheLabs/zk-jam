import type { ButtonHTMLAttributes, HTMLAttributes, InputHTMLAttributes, ReactNode } from "react";

export function Card({ className = "", ...props }: HTMLAttributes<HTMLDivElement>) { return <section className={`card ${className}`} {...props} />; }
export function Button({ className = "", ...props }: ButtonHTMLAttributes<HTMLButtonElement>) { return <button className={`button ${className}`} {...props} />; }
export function Badge({ children, className = "" }: { children: ReactNode; className?: string }) { return <span className={`badge ${className}`}>{children}</span>; }
export function Slider(props: InputHTMLAttributes<HTMLInputElement>) { return <input type="range" {...props} />; }
export function FieldLabel({ children }: { children: ReactNode }) { return <span className="field-label">{children}</span>; }
