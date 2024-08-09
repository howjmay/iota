// Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

import { useEffect, useRef, useState } from 'react';
import { InputWrapper, InputWrapperProps } from './InputWrapper';
import {
    BORDER_CLASSES,
    INPUT_CLASSES,
    INPUT_TEXT_CLASSES,
    INPUT_PLACEHOLDER_CLASSES,
} from './input.classes';
import cx from 'classnames';
import { ButtonUnstyled } from '../../atoms/button/ButtonUnstyled';
import { VisibilityOff, VisibilityOn } from '@iota/ui-icons';

type InputPickedProps = Pick<
    React.TextareaHTMLAttributes<HTMLTextAreaElement>,
    | 'maxLength'
    | 'minLength'
    | 'rows'
    | 'autoFocus'
    | 'name'
    | 'required'
    | 'placeholder'
    | 'disabled'
    | 'id'
>;

interface TextFieldBaseProps extends InputPickedProps, InputWrapperProps {
    /**
     * Shows a label with the text above the input field.
     */
    label?: string;
    /**
     * Shows a caption with the text below the input field.
     */
    caption?: string;
    /**
     * Error Message. Overrides the caption.
     */
    errorMessage?: string;
    /**
     * Callback function that is called when the input field value changes
     */
    onChange?: (value: string, name?: string) => void;
    /**
     * Amount counter that is shown at the side of the caption text.
     */
    amountCounter?: string | number;
    /**
     * Shows toggle button to show/hide the content of the input field
     */
    isVisibilityToggleEnabled?: boolean;
    /**
     * Ref for the input field
     */
    ref?: React.RefObject<HTMLTextAreaElement>;
    /**
     * Is the content of the input visible
     */
    isContentVisible?: boolean;
    /**
     * Value of the input field
     */
    value?: string;
    /**
     * If true the textarea is resizable vertically
     */
    isResizeEnabled?: boolean;
}

export function TextArea({
    name,
    label,
    placeholder,
    caption,
    disabled,
    errorMessage,
    onChange,
    value,
    amountCounter,
    isVisibilityToggleEnabled,
    isResizeEnabled,
    rows = 3,
    autoFocus,
    required,
    maxLength,
    minLength,
    isContentVisible,
    ref,
    id,
}: TextFieldBaseProps) {
    const fallbackRef = useRef<HTMLTextAreaElement>(null);
    const inputRef = ref ?? fallbackRef;

    const [isInputContentVisible, setIsInputContentVisible] = useState<boolean>(
        isContentVisible ?? true,
    );

    useEffect(() => {
        setIsInputContentVisible(isContentVisible ?? true);
    }, [isContentVisible]);

    function onToggleButtonClick() {
        setIsInputContentVisible((prev) => !prev);
    }

    function handleOnChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
        if (isInputContentVisible) {
            onChange?.(e.target.value, e.target.name);
        }
    }

    return (
        <InputWrapper
            label={label}
            caption={caption}
            disabled={disabled}
            errorMessage={errorMessage}
            amountCounter={amountCounter}
            required={required}
        >
            <div className="relative">
                <textarea
                    disabled={disabled || !isInputContentVisible}
                    placeholder={placeholder}
                    required={required}
                    id={id}
                    name={name}
                    rows={rows}
                    autoFocus={autoFocus}
                    ref={inputRef}
                    onChange={handleOnChange}
                    className={cx(
                        'peer block min-h-[50px]',
                        BORDER_CLASSES,
                        INPUT_CLASSES,
                        INPUT_TEXT_CLASSES,
                        INPUT_PLACEHOLDER_CLASSES,
                        isInputContentVisible && isResizeEnabled ? 'resize-y' : 'resize-none',
                        !isInputContentVisible &&
                            'not-visible select-none text-transparent dark:text-transparent',
                    )}
                    value={isInputContentVisible ? value : ''}
                    maxLength={maxLength}
                    minLength={minLength}
                />
                {!isInputContentVisible && (
                    <div className="absolute left-0 top-0 flex h-full w-full flex-col items-stretch gap-y-2 px-md py-sm peer-[.not-visible]:select-none">
                        <VisibilityOffBar rows={3} halfWidthRow={3} />
                    </div>
                )}

                {isVisibilityToggleEnabled && (
                    <span className="absolute bottom-4 right-4 flex">
                        <ButtonUnstyled
                            onClick={onToggleButtonClick}
                            className="text-neutral-10 dark:text-neutral-92"
                        >
                            {isInputContentVisible ? <VisibilityOn /> : <VisibilityOff />}
                        </ButtonUnstyled>
                    </span>
                )}
            </div>
        </InputWrapper>
    );
}

interface VisibilityOffBarProps {
    rows: number;
    halfWidthRow?: number;
}

function VisibilityOffBar({ rows, halfWidthRow }: VisibilityOffBarProps) {
    return (
        <>
            {Array.from({ length: rows }).map((_, index) => {
                const isHalfWidth = halfWidthRow === index + 1;
                const width = isHalfWidth ? 'w-1/2' : 'w-full';
                return (
                    <div
                        key={index}
                        className={`h-2.5 rounded bg-neutral-92/60 dark:bg-neutral-10/60 ${width}`}
                    />
                );
            })}
        </>
    );
}
