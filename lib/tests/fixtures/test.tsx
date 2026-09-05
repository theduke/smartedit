interface MyProps {
    value: string;
}

export function MyComponent(props: MyProps) {
    return <div>{props.value}</div>;
}
